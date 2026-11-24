//! File-operation UI: request planning, sheets, Activity and quit coordination.
//!
//! `Sheet` itself, `close_sheet`, onboarding and the shared focus surfaces stay
//! in `window.rs`; this module owns the operation content and dispatch. The
//! app-wide queue and its active job live in `operations.rs`.

use super::*;
use crate::actions::MoveToTrash;
use crate::operations::{self, JobPhase, Maintenance, Origin, RenameMap, RequestId, StopIntent};
use crow_core::motion::Fx;
use crow_fs::ops::{ConflictKey, Resolution};

use crate::chrome::cycle_focus;

/// Quit, unless work is pending: then the active Crow window asks first. With no
/// window open, one opens at the launch folder to ask.
pub(crate) fn quit(cx: &mut App) {
    if !cx.global::<Operations>().has_pending() {
        cx.quit();
        return;
    }
    let target = cx
        .active_window()
        .and_then(|w| w.downcast::<CrowWindow>())
        .or_else(|| cx.windows().into_iter().find_map(|w| w.downcast::<CrowWindow>()));
    match target {
        // Deferred: ⌘Q and the palette dispatch from inside that window's own update.
        Some(handle) => cx.defer(move |cx| {
            if handle.update(cx, |this, window, cx| this.ask_to_stop(StopIntent::Quit, window, cx)).is_err() {
                stop_then(StopIntent::Quit, None, cx);
            }
        }),
        None => match open(launch_folder(cx), false, None, cx) {
            Some(handle) => cx.defer(move |cx| {
                if handle.update(cx, |this, window, cx| this.ask_to_stop(StopIntent::Quit, window, cx)).is_err() {
                    stop_then(StopIntent::Quit, None, cx);
                }
            }),
            None => stop_then(StopIntent::Quit, None, cx),
        },
    }
}

/// Cancel every waiting request and the head, wait for the active worker's
/// terminal cleanup, then quit or close the window.
fn stop_then(intent: StopIntent, handle: Option<gpui::WindowHandle<CrowWindow>>, cx: &mut App) {
    if cx.global::<Operations>().stopping {
        return;
    }
    operations::cancel_all(cx);
    cx.spawn(async move |cx| {
        while cx.update(|cx| !cx.global::<Operations>().is_idle()) {
            cx.background_executor().timer(std::time::Duration::from_millis(30)).await;
        }
        cx.update(|cx| match intent {
            StopIntent::Quit => cx.quit(),
            StopIntent::Close => {
                if let Some(handle) = handle {
                    handle
                        .update(cx, |this, window, cx| {
                            this.pane().update(cx, |pane, cx| pane.release_listings(cx));
                            window.remove_window();
                        })
                        .ok();
                }
                cx.global_mut::<Operations>().stopping = false;
            }
        });
    })
    .detach();
}

impl CrowWindow {
    /// Whether the right sidebar is open on its Activity tab.
    pub(super) fn activity_open(&self, cx: &App) -> bool {
        let info = self.info.read(cx);
        info.visible() && info.tab() == InfoTab::Activity
    }

    pub(super) fn activity_owns_focus(&self, window: &Window, cx: &App) -> bool {
        self.sheet.is_none() && self.activity_open(cx) && self.activity_focus.contains_focused(window, cx)
    }

    pub(super) fn toggle_activity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sheet.is_some() {
            return;
        }
        if self.activity_owns_focus(window, cx) {
            self.focus_center(window, cx);
        }
        let was_visible = self.info.read(cx).visible();
        let show = !self.activity_open(cx);
        self.info.update(cx, |info, cx| {
            info.set_visible(show, cx);
            if show {
                info.set_tab(InfoTab::Activity, cx);
            }
        });
        self.begin_pane_slide(was_visible, cx);
        if show {
            window.focus(&self.activity_focus, cx);
        }
        cx.notify();
    }

    /// A red close or Cmd-W on the last tab: ask before cutting off pending work.
    pub(super) fn window_should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !operations::is_last_crow_window(window.window_handle().window_id(), cx) {
            return true;
        }
        if cx.global::<Operations>().stopping {
            return false;
        }
        if !cx.global::<Operations>().has_pending() {
            return true;
        }
        self.ask_to_stop(StopIntent::Close, window, cx);
        false
    }

    pub(super) fn ask_to_stop(&mut self, intent: StopIntent, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.sheet, Some(Sheet::StopOperation { .. })) {
            return;
        }
        // The stop sheet replaces a conflict sheet rather than stacking on it; the
        // job keeps waiting if the user keeps working.
        if matches!(self.sheet, Some(Sheet::Conflict { .. }))
            && let Some(prompt) = cx.global_mut::<Operations>().prompt.as_mut()
        {
            prompt.host = None;
        }
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let buttons = (0..2).map(|_| cx.focus_handle()).collect();
        self.sheet = Some(Sheet::StopOperation { focus, intent, buttons });
        // A conflict sheet in another window must not stack with this question.
        cx.defer(|cx| operations::dismiss_prompt_sheets(None, cx));
        window.activate_window();
        cx.notify();
    }

    /// Whether `tab` is the active tab and still shows `dir`.
    pub(crate) fn origin_is_current(&self, origin: &Origin, cx: &App) -> bool {
        let active = self.tabs.iter().nth(self.tabs.active_index()).map(|(id, _)| id);
        active == Some(origin.tab)
            && self.pane().read(cx).current_dir(cx) == origin.dir
            && self.operation_surface(cx) == origin.surface
    }

    fn operation_surface(&self, cx: &App) -> crate::plugins::SurfaceId {
        self.active_search(cx)
            .map_or_else(|| self.pane().read(cx).current_plugin_surface(cx), |search| search.read(cx).plugin_surface())
    }

    fn origin(&self, window: &Window, cx: &App) -> Option<Origin> {
        let pane = self.pane();
        let tab = self.tabs.iter().find(|(_, candidate)| *candidate == pane).map(|(tab, _)| tab)?;
        let window = window.window_handle().downcast::<CrowWindow>()?;
        Some(Origin { window, tab, dir: pane.read(cx).current_dir(cx), surface: self.operation_surface(cx) })
    }

    /// A permanent delete asks first; everything else enters the app-wide queue.
    pub(super) fn run_request(&mut self, request: Request, window: &mut Window, cx: &mut Context<Self>) {
        if let Request::DeletePermanently { paths } = request {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            let buttons = (0..2).map(|_| cx.focus_handle()).collect();
            self.sheet = Some(Sheet::ConfirmDelete { paths, focus, buttons });
            cx.notify();
            return;
        }
        operations::enqueue(request, self.origin(window, cx), cx);
        cx.notify();
    }

    /// The head began: row effects belong to the moment the engine starts, not
    /// to the key press that queued it.
    pub(crate) fn job_started(&mut self, request: &Request, cx: &mut Context<Self>) {
        if matches!(request, Request::NewFolder { .. } | Request::NewFile { .. }) {
            // The new item must be visible: a filter that hides it would hide the
            // rename field that follows (findings #10).
            self.pane().update(cx, |pane, cx| pane.clear_filter_for_new_item(cx));
        }
        let pane = self.pane().downgrade();
        match request {
            Request::Trash { paths } => {
                pane.update(cx, |pane, cx| pane.start_effect(paths, Fx::Trash, cx)).ok();
            }
            Request::Move { paths, into } => {
                pane.update(cx, |pane, cx| pane.start_effect(paths, Fx::MoveOut, cx)).ok();
                // The folder the items land in pulses, in the list and in the sidebar.
                let into = into.clone();
                pane.update(cx, |pane, cx| pane.start_effect(std::slice::from_ref(&into), Fx::Receive, cx)).ok();
                self.sidebar.update(cx, |sidebar, cx| sidebar.pulse(into, cx));
            }
            _ => {}
        }
    }

    /// Keep selection and cursor on the same items after a batch rename, in the
    /// folder pane, the saved or tag session and the window search.
    fn apply_rename_map(&mut self, map: &RenameMap, cursor: Option<&Path>, cx: &mut Context<Self>) {
        let pane = self.pane().downgrade();
        pane.update(cx, |pane, cx| pane.apply_rename_map(map, cursor, cx)).ok();
        if let Some(search) = self.search.clone() {
            search.update(cx, |search, cx| search.apply_rename_map(map, cx));
        }
    }

    /// The head reached a terminal state: select what it produced only while the
    /// original tab still shows the original folder.
    pub(crate) fn job_finished(
        &mut self,
        request: &Request,
        results: Vec<PathBuf>,
        rename: bool,
        failed: bool,
        cx: &mut Context<Self>,
    ) {
        if let Request::BatchRename { moves } = request
            && !failed
        {
            let map: std::collections::HashMap<PathBuf, PathBuf> = moves
                .iter()
                .filter(|entry| entry.source != entry.target)
                .map(|entry| (entry.source.clone(), entry.target.clone()))
                .collect();
            let cursor = cx.global_mut::<Operations>().batch_cursor.take();
            self.apply_rename_map(&map, cursor.as_deref(), cx);
            return;
        }
        let pane = self.pane().downgrade();
        if failed
            && let Some(paths) = match request {
                Request::Trash { paths }
                | Request::Move { paths, .. }
                | Request::Copy { paths, .. }
                | Request::Duplicate { paths } => Some(paths.clone()),
                _ => None,
            }
        {
            // Whatever is still there refuses.
            pane.update(cx, |pane, cx| pane.start_effect(&paths, Fx::Refuse, cx)).ok();
        }
        // With `folder_target = "selected"` a new item can land in a folder that is not
        // listed: nothing to select or rename there, so say where it went.
        let (results, rename) = match request {
            Request::NewFolder { parent } | Request::NewFile { parent }
                if !failed && *parent != self.pane().read(cx).current_dir(cx) =>
            {
                if let Some(created) = results.first() {
                    let name = created.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                    let folder = parent.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                    operations::message(format!("Created “{name}” in {folder}"), false, cx);
                    cx.notify();
                }
                (Vec::new(), false)
            }
            _ => (results, rename),
        };
        pane.update(cx, |pane, cx| {
            pane.show_results(results, rename, cx);
            pane.refresh_saved(cx);
        })
        .ok();
    }

    /// Show the app-wide conflict prompt in this window. Returns false when this
    /// window already owns a sheet or no prompt is waiting.
    pub(crate) fn show_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.sheet.is_some() {
            return false;
        }
        let Some(prompt) = cx.global::<Operations>().prompt.clone() else { return false };
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.sheet = Some(Sheet::Conflict {
            focus,
            controls: std::array::from_fn(|_| cx.focus_handle()),
            job: prompt.job,
            key: prompt.key(),
            choice: Resolution::KeepBoth,
            all: false,
        });
        if let Some(prompt) = cx.global_mut::<Operations>().prompt.as_mut() {
            prompt.host = Some(window.window_handle().window_id());
        }
        window.activate_window();
        cx.notify();
        true
    }

    /// A prompt's job advanced or moved windows: keep a matching sheet current.
    pub(crate) fn refresh_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(prompt) = cx.global::<Operations>().prompt.clone() else { return false };
        match &self.sheet {
            Some(Sheet::Conflict { job, key, .. }) if *job == prompt.job && *key == prompt.key() => {
                cx.notify();
                true
            }
            Some(Sheet::Conflict { .. }) => {
                // A newer attempt (or job) replaced the question; show that one.
                retire!(self, cx, sheet);
                self.show_conflict(window, cx)
            }
            Some(_) => false,
            None => self.show_conflict(window, cx),
        }
    }

    /// Drop a conflict sheet without answering its job. `Some(key)` only drops the
    /// matching attempt; `None` drops any.
    pub(crate) fn dismiss_conflict_sheet(
        &mut self,
        key: Option<ConflictKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let matches = match &self.sheet {
            Some(Sheet::Conflict { key: current, .. }) => key.is_none_or(|key| key == *current),
            _ => false,
        };
        if !matches {
            return;
        }
        retire!(self, cx, sheet);
        if let Some(prompt) = cx.global_mut::<Operations>().prompt.as_mut()
            && prompt.host == Some(window.window_handle().window_id())
        {
            prompt.host = None;
        }
        self.focus_center(window, cx);
        cx.notify();
    }

    pub(super) fn undo(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        // Undo ends the cut mark; the clipboard itself stays for a later paste.
        if let Some(clipboard) = &mut cx.global_mut::<Operations>().clipboard {
            clipboard.marked = false;
        }
        self.notify_views(cx);
        let pane = self.pane().downgrade();
        let search = self.search.clone();
        operations::undo(index, cx, move |restored, renamed, cx| {
            finish_reversal(&pane, search, restored, renamed, cx);
        });
    }

    pub(super) fn redo(&mut self, cx: &mut Context<Self>) {
        let pane = self.pane().downgrade();
        let search = self.search.clone();
        operations::redo(cx, move |results, renamed, cx| {
            finish_reversal(&pane, search, results, renamed, cx);
        });
    }

    /// ⌘⇧⌫, File ▸ Empty Trash…, or the palette. Counts off the main thread
    /// first: an empty Trash says so without asking.
    pub(super) fn empty_trash(&mut self, _: &EmptyTrash, window: &mut Window, cx: &mut Context<Self>) {
        if self.sheet.is_some() {
            return;
        }
        if !cx.global::<Operations>().is_idle() {
            operations::message("Another operation is still running", true, cx);
            cx.notify();
            return;
        }
        let home = cx.global::<Ui>().home.clone();
        let volumes: Vec<PathBuf> = cx.global::<Ui>().volumes.iter().map(|volume| volume.path.clone()).collect();
        // A fuzz run must never reach the user's real Trash on a mounted volume.
        let jail = cx.global::<Operations>().jail().map(Path::to_path_buf);
        let jailed = jail.is_some();
        cx.spawn_in(window, async move |this, cx| {
            let (folders, count) = cx
                .background_spawn(async move {
                    let folders = crate::trash::in_jail(crate::trash::folders(&home, &volumes), jail.as_deref());
                    let count = crow_fs::trash::count(&folders);
                    (folders, count)
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                if this.overlay_open() {
                    return;
                }
                // A Trash macOS will not let Crow list is unknown, never empty.
                let denied = !count.denied.is_empty();
                if denied && jailed {
                    operations::message(crate::trash::denied_message(&count.denied), true, cx);
                    cx.notify();
                    return;
                }
                if count.items == 0 && !denied {
                    operations::message("The Trash is already empty.", false, cx);
                    cx.notify();
                    return;
                }
                let focus = cx.focus_handle();
                window.focus(&focus, cx);
                let buttons = (0..if denied { 3 } else { 2 }).map(|_| cx.focus_handle()).collect();
                this.sheet = Some(Sheet::ConfirmEmptyTrash { folders, finder: denied, focus, buttons });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Remove every folder's items in the background, then report and play the
    /// sound. Failures leave the rest removed and name what stayed.
    fn run_empty_trash(&mut self, folders: Vec<PathBuf>, finder: bool, cx: &mut Context<Self>) {
        if !operations::begin_maintenance(Maintenance::EmptyTrash, cx) {
            return;
        }
        let effects = crate::effects::effects(cx);
        cx.spawn(async move |_, cx| {
            let (text, is_error, play) = cx
                .background_spawn(async move {
                    if finder {
                        return match effects.finder_empty_trash() {
                            Ok(()) => ("Emptied the Trash with Finder.".to_owned(), false, true),
                            Err(error) => (crate::trash::finder_failure(&error), true, false),
                        };
                    }
                    let report = crow_fs::trash::empty(&folders);
                    let (text, is_error) = crate::trash::message(&report);
                    (text, is_error, report.removed > 0)
                })
                .await;
            cx.update(|cx| {
                operations::log_maintenance(&text, is_error, cx);
                operations::message(text, is_error, cx);
                if play {
                    crate::feedback::play(crow_core::motion::Event::EmptyTrash, cx);
                }
                operations::end_maintenance(cx);
            });
        })
        .detach();
    }

    pub(super) fn confirm_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.sheet, Some(Sheet::BatchRename(_))) {
            self.submit_batch_rename(window, cx);
            return;
        }
        if matches!(self.sheet, Some(Sheet::SavedQuery(_))) {
            self.confirm_saved_query_form(window, cx);
            return;
        }
        // The answer is read from the sheet, then the sheet retires (its picture fades
        // while the operation starts at once).
        match &self.sheet {
            Some(Sheet::Conflict { job, key, choice, all, .. }) => {
                let (job, key, choice, all) = (*job, *key, *choice, *all);
                retire!(self, cx, sheet);
                operations::resolve_prompt(job, key, choice, all, cx);
            }
            Some(Sheet::ConfirmDelete { paths, .. }) => {
                let paths = paths.clone();
                retire!(self, cx, sheet);
                operations::enqueue(Request::DeletePermanently { paths }, self.origin(window, cx), cx);
            }
            Some(Sheet::ConfirmEmptyTrash { folders, finder, .. }) => {
                let (folders, finder) = (folders.clone(), *finder);
                retire!(self, cx, sheet);
                self.run_empty_trash(folders, finder, cx);
            }
            Some(Sheet::StopOperation { intent, .. }) => {
                let intent = *intent;
                retire!(self, cx, sheet);
                let handle = window.window_handle().downcast::<CrowWindow>();
                stop_then(intent, handle, cx);
                return;
            }
            Some(Sheet::SavedQuery(_) | Sheet::BatchRename(_)) => {
                retire!(self, cx, sheet);
            }
            Some(Sheet::Onboarding { no, .. }) => {
                // Return activates the focused choice; the panel focus means the
                // default, Show tips.
                let show_tips = !no.is_focused(window);
                self.choose_tips(show_tips, window, cx);
                return;
            }
            None => {}
        }
        self.focus_center(window, cx);
        cx.notify();
    }

    fn set_conflict_choice(&mut self, choice: Resolution, all: bool, cx: &mut Context<Self>) {
        if let Some(Sheet::Conflict { choice: current, all: every, .. }) = &mut self.sheet {
            *current = choice;
            *every = all;
            cx.notify();
        }
    }

    fn activate_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A confirmation answers with the button that has focus: Cancel cancels,
        // Open Full Disk Access opens its page, and the last one (or none) confirms.
        if let Some(buttons) = self.sheet.as_ref().and_then(Sheet::buttons)
            && let Some(index) = buttons.iter().position(|button| button.is_focused(window))
            && index + 1 < buttons.len()
        {
            if index > 0 {
                cx.open_url(crate::trash::PRIVACY_SETTINGS_URL);
            }
            self.close_sheet(window, cx);
            return;
        }
        if let Some(Sheet::Conflict { controls, choice, all, .. }) = &self.sheet {
            match controls.iter().position(|control| control.is_focused(window)) {
                Some(index @ 0..=2) => {
                    let choice = [Resolution::KeepBoth, Resolution::Replace, Resolution::Skip][index];
                    self.set_conflict_choice(choice, *all, cx);
                    return;
                }
                Some(3) => {
                    self.set_conflict_choice(*choice, !*all, cx);
                    return;
                }
                Some(4) => {
                    self.close_sheet(window, cx);
                    return;
                }
                _ => {}
            }
        }
        self.confirm_sheet(window, cx);
    }

    pub(super) fn render_sheet(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if matches!(self.sheet, Some(Sheet::BatchRename(_))) {
            return Some(self.render_batch_rename(window, cx));
        }
        let sheet = self.sheet.as_ref()?;
        let theme = cx.global::<Ui>().theme.clone();
        let (fg, muted, accent) = (theme.role("fg"), theme.role("fg-muted"), theme.role("accent"));
        let trash_hint = match keys::text(&MoveToTrash, KeyScope::List, cx) {
            Some(keys) => format!("This can't be undone. Move to Trash ({keys}) can."),
            None => "This can't be undone. Move to Trash can.".to_owned(),
        };
        let button = |id: &'static str, label: String, primary: bool, focus: Option<&FocusHandle>| {
            let focused = focus.is_some_and(|focus| focus.is_focused(window));
            crate::chrome::sheet_button(id, label, primary, &theme)
                .when_some(focus, |button, focus| button.track_focus(focus).tab_stop(true))
                .when(focused, |button| button.shadow(cursor_ring(&theme)))
        };
        let (title, body): (String, gpui::AnyElement) = match sheet {
            Sheet::ConfirmDelete { paths, buttons, .. } => {
                let what = match &paths[..] {
                    [one] => {
                        let name = one.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                        format!("“{}”", crate::style::display_text(&name))
                    }
                    many => format!("{} items", many.len()),
                };
                (
                    format!("Delete {what} permanently?"),
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(div().text_color(color(muted)).child(trash_hint))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    button("sheet-cancel", "Cancel".into(), false, Some(&buttons[0]))
                                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                                )
                                .child(
                                    button("sheet-confirm", "Delete".into(), true, Some(&buttons[1]))
                                        .on_click(cx.listener(|this, _, window, cx| this.confirm_sheet(window, cx))),
                                ),
                        )
                        .into_any_element(),
                )
            }
            Sheet::ConfirmEmptyTrash { finder, buttons, .. } => {
                let finder = *finder;
                let confirm = buttons.last().expect("a confirming button");
                (
                    if finder { "Empty the Trash with Finder?" } else { "Empty the Trash?" }.to_owned(),
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(div().text_color(color(muted)).child(if finder {
                            "macOS does not let Crow read the Trash without Full Disk Access. Finder can empty it, or you can allow Crow access. Every item in the Trash will be deleted. This can't be undone."
                        } else {
                            "Every item in the Trash, on this Mac and your other disks, will be deleted. This can't be undone."
                        }))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    button("sheet-cancel", "Cancel".into(), false, Some(&buttons[0]))
                                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                                )
                                .when(finder, |row| {
                                    row.child(
                                        button(
                                            "sheet-full-disk-access",
                                            "Open Full Disk Access…".into(),
                                            false,
                                            Some(&buttons[1]),
                                        )
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                cx.open_url(crate::trash::PRIVACY_SETTINGS_URL);
                                                this.close_sheet(window, cx);
                                            })),
                                    )
                                })
                                .child(
                                    button(
                                        "sheet-confirm",
                                        if finder { "Empty with Finder" } else { "Empty Trash" }.into(),
                                        true,
                                        Some(confirm),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| this.confirm_sheet(window, cx))),
                                ),
                        )
                        .into_any_element(),
                )
            }
            Sheet::StopOperation { intent, buttons, .. } => {
                let label = operations::stop_label(cx);
                let queued = cx.global::<Operations>().queued_count();
                let (verb, confirm) = match intent {
                    StopIntent::Quit => ("Quitting", "Stop and Quit"),
                    StopIntent::Close => ("Closing", "Stop and Close"),
                };
                let waiting = if queued == 0 {
                    String::new()
                } else if queued == 1 {
                    " One more request is waiting; it is cancelled too.".to_owned()
                } else {
                    format!(" {queued} more requests are waiting; they are cancelled too.")
                };
                (
                    format!("{label} is still in progress"),
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(div().text_color(color(muted)).child(format!(
                            "{verb} would cut it off. Stop it cleanly first? Items already finished stay (Undo can reverse them); the one in progress is left out, never half done.{waiting}"
                        )))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    button("sheet-cancel", "Keep Running".into(), false, Some(&buttons[0]))
                                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                                )
                                .child(
                                    button("sheet-confirm", confirm.into(), true, Some(&buttons[1]))
                                        .on_click(cx.listener(|this, _, window, cx| this.confirm_sheet(window, cx))),
                                ),
                        )
                        .into_any_element(),
                )
            }
            Sheet::SavedQuery(form) => (
                if form.id.is_some() { "Edit Saved Search" } else { "Save Search" }.to_owned(),
                self.render_saved_query_form(form, window, cx),
            ),
            Sheet::BatchRename(_) => unreachable!("batch rename renders above"),
            Sheet::Onboarding { show, no, .. } => (
                "Show tips as you go?".to_owned(),
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_color(color(muted)).child(
                        "Crow can point out a few keys the first time they matter. You can change this later in Settings.",
                    ))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                button("onboarding-no", "No tips".into(), false, Some(no))
                                    .on_click(cx.listener(|this, _, window, cx| this.choose_tips(false, window, cx))),
                            )
                            .child(
                                button("onboarding-show", "Show tips".into(), true, Some(show))
                                    .on_click(cx.listener(|this, _, window, cx| this.choose_tips(true, window, cx))),
                            ),
                    )
                    .into_any_element(),
            ),
            Sheet::Conflict { job, key, choice, all, controls, .. } => {
                let prompt = cx
                    .global::<Operations>()
                    .prompt
                    .clone()
                    .filter(|prompt| prompt.job == *job && prompt.key() == *key);
                if let Some(prompt) = prompt {
                    let conflict = &prompt.conflict;
                    let name = conflict.target.file_name().map_or_else(String::new, |name| {
                        crate::style::display_text(&name.to_string_lossy()).into_owned()
                    });
                    let into = conflict
                        .target
                        .parent()
                        .map(crate::style::display_name)
                        .unwrap_or_default();
                    let reason = match conflict.reason {
                        crow_fs::ops::ConflictReason::Exists => "This item already exists in the folder.",
                        crow_fs::ops::ConflictReason::Appeared => "This item appeared while the operation was running.",
                        crow_fs::ops::ConflictReason::Changed => {
                            "The item at this location changed. Choose again."
                        }
                    };
                    let choice = *choice;
                    let all = *all;
                    let option = |value: Resolution, label: &'static str, focus: &FocusHandle, cx: &mut Context<Self>| {
                        let on = value == choice;
                        div()
                            .id(("conflict-choice", value as usize))
                            .debug_selector(move || format!("conflict-choice-{}", value as usize))
                            .track_focus(focus)
                            .focus_visible(|style| style.shadow(cursor_ring(&theme)))
                            .role(Role::RadioButton)
                            .aria_label(label)
                            .aria_selected(on)
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .text_size(px(12.))
                            .when(on, |option| option.bg(active_selection(&theme)).text_color(color(fg)))
                            .when(!on, |option| option.bg(color(theme.role("surface-raised"))))
                            .on_click(cx.listener(move |this, _, _, cx| this.set_conflict_choice(value, all, cx)))
                            .child(label)
                    };
                    let apply_all = div()
                        .id("conflict-all")
                        .debug_selector(|| "conflict-all".into())
                        .track_focus(&controls[3])
                        .focus_visible(|style| style.shadow(cursor_ring(&theme)))
                        .role(Role::CheckBox)
                        .aria_label("Apply to all remaining conflicts")
                        .aria_toggled(if all { gpui::Toggled::True } else { gpui::Toggled::False })
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(color(muted))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_conflict_choice(choice, !all, cx)))
                        .child(div().w(px(14.)).h(px(14.)).rounded_sm().border_1().border_color(color(theme.role("border"))).when(
                            all,
                            |box_| box_.bg(color(accent)),
                        ))
                        .child("Apply to all remaining conflicts");
                    (
                        format!("“{name}” already exists in “{into}”"),
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(div().text_color(color(muted)).child(reason))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(option(Resolution::KeepBoth, "Keep Both", &controls[0], cx))
                                    .child(option(Resolution::Replace, "Replace", &controls[1], cx))
                                    .child(option(Resolution::Skip, "Skip", &controls[2], cx)),
                            )
                            .child(div().text_size(px(12.)).text_color(color(muted)).child(
                                "Keep Both adds a number to the new name. Replace moves the existing item to the Trash.",
                            ))
                            .child(apply_all)
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        button("sheet-cancel", "Decide Later".into(), false, Some(&controls[4]))
                                            .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                                    )
                                    .child(
                                        button("sheet-confirm", "Continue".into(), true, Some(&controls[5]))
                                            .on_click(cx.listener(|this, _, window, cx| this.confirm_sheet(window, cx))),
                                    ),
                            )
                            .into_any_element(),
                    )
                } else {
                    (
                        "This conflict is no longer waiting".to_owned(),
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(div().text_color(color(muted)).child("Another answer or a stop already settled it."))
                            .child(
                                div().flex().justify_end().child(
                                    button("sheet-cancel", "Close".into(), false, None)
                                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                                ),
                            )
                            .into_any_element(),
                    )
                }
            }
        };
        let sheet_tab = cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
            // Space presses the focused button, as Return does.
            if event.keystroke.key == "space" && event.keystroke.modifiers == gpui::Modifiers::default() {
                let on_button = this.sheet.as_ref().is_some_and(|sheet| match sheet {
                    Sheet::Conflict { controls, .. } => controls.iter().any(|control| control.is_focused(window)),
                    sheet => sheet.buttons().is_some_and(|buttons| buttons.iter().any(|b| b.is_focused(window))),
                });
                if on_button {
                    this.activate_sheet(window, cx);
                    cx.stop_propagation();
                }
                return;
            }
            if event.keystroke.key != "tab" {
                return;
            }
            let forward = !event.keystroke.modifiers.shift;
            if let Some(buttons) = this.sheet.as_ref().and_then(Sheet::buttons) {
                cycle_focus(buttons, forward, window, cx);
                cx.notify();
            } else if let Some(Sheet::Conflict { controls, .. }) = &this.sheet {
                cycle_focus(controls, forward, window, cx);
                cx.notify();
            } else if matches!(this.sheet, Some(Sheet::BatchRename(_))) {
                this.batch_rename_tab(forward, window, cx);
            } else {
                this.move_onboarding_focus(forward, window, cx);
            }
            cx.stop_propagation();
        });
        Some(
            chrome::sheet(
                chrome::sheet_panel(
                    "sheet",
                    if matches!(sheet, Sheet::SavedQuery(_)) { SheetWidth::Wide } else { SheetWidth::Narrow },
                    &theme,
                )
                .aria_label(title.clone())
                // keys::context_stack (KeyScope::Sheet) mirrors this.
                .key_context("Sheet")
                .when(!matches!(sheet, Sheet::SavedQuery(_)), |panel| panel.track_focus(sheet.focus()))
                // The form is the tallest sheet: it scrolls inside a short or narrow window.
                .when(matches!(sheet, Sheet::SavedQuery(_)), |panel| {
                    panel
                        .max_w(window.viewport_size().width - px(24.))
                        .max_h(window.viewport_size().height - px(50.))
                        .overflow_y_scroll()
                })
                .on_action(
                    cx.listener(|this, _: &crate::actions::SheetConfirm, window, cx| this.activate_sheet(window, cx)),
                )
                .on_action(
                    cx.listener(|this, _: &crate::actions::SheetCancel, window, cx| this.close_sheet(window, cx)),
                )
                .when(!matches!(sheet, Sheet::SavedQuery(_) | Sheet::BatchRename(_)), |panel| {
                    panel.on_key_down(sheet_tab)
                })
                .p_4()
                // The first-launch question sits a little lower than the other
                // sheets, so it reads as an invitation rather than an alert.
                .when(matches!(sheet, Sheet::Onboarding { .. }), |panel| panel.mt_2())
                .flex()
                .flex_col()
                .gap_3()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                .child(body),
                &theme,
                cx.global::<Ui>().feedback.mode,
                self.exit_fade.get(),
                cx.listener(|this, _, window, cx| this.close_sheet(window, cx)),
            )
            .into_any_element(),
        )
    }

    /// Active and queued jobs first, then recent terminal outcomes newest first.
    pub(super) fn render_activity(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let muted = color(theme.role("fg-muted"));
        let border = color(theme.role("border"));
        let (active, queued, running, prompt_job, idle, emptying_trash) = {
            let ops = cx.global::<Operations>();
            let active = ops.active.as_ref().map(|active| {
                (active.id, active.label.clone(), active.phase, active.pause_supported, active.conflict.is_some())
            });
            let queued: Vec<(RequestId, String)> =
                ops.queue.iter().map(|queued| (queued.id, queued.label.clone())).collect();
            let running = ops.running().map(|mut running| {
                running.queued = 0;
                running
            });
            (
                active,
                queued,
                running,
                ops.prompt.as_ref().map(|prompt| prompt.job),
                ops.is_idle(),
                ops.maintenance == Some(Maintenance::EmptyTrash),
            )
        };
        let entries: Vec<(usize, String, String, bool)> = cx
            .global::<Operations>()
            .log
            .iter()
            .enumerate()
            .rev()
            .take(50)
            .map(|(i, e)| {
                let when: chrono::DateTime<chrono::Local> = e.at.into();
                let state = match (&e.error, e.cancelled, e.undone) {
                    (_, _, true) => "undone".to_owned(),
                    (Some(err), _, _) => format!("stopped: {err}"),
                    (None, true, _) => "cancelled".to_owned(),
                    _ => String::new(),
                };
                // Rename labels carry both names; errors carry the path at fault.
                let label = crate::style::display_text(&e.record.label).into_owned();
                let state = crate::style::display_text(&state).into_owned();
                let when = when.format("%H:%M").to_string();
                let when = if state.is_empty() { when } else { format!("{when} {state}") };
                (i, label, when, e.record.is_undoable() && !e.undone)
            })
            .collect();
        let mut previous = std::mem::take(&mut self.activity_controls);
        let mut controls = Vec::new();
        let mut pill = |id: (&'static str, u64), label: &'static str, primary: bool, cx: &Context<Self>| {
            let debug = format!("{}-{}", id.0, id.1);
            let focus = previous.remove(&id).unwrap_or_else(|| cx.focus_handle());
            controls.push(focus.clone());
            self.activity_controls.insert(id, focus.clone());
            div()
                .id(id)
                .debug_selector(move || debug.clone())
                .track_focus(&focus)
                .focus_visible(|style| style.shadow(cursor_ring(&theme)))
                .role(Role::Button)
                .aria_label(label)
                .px_2()
                .py(px(2.))
                .rounded_sm()
                .text_size(px(11.))
                .bg(color(if primary { theme.role("accent") } else { theme.role("surface-raised") }))
                .text_color(color(if primary { theme.role("on-accent") } else { theme.role("fg") }))
                .child(label)
        };
        let running_label = active.as_ref().map(|(_, label, ..)| operations::running_label(label));
        let active_row = active.map(|(id, label, phase, pause_supported, has_conflict)| {
            let detail = running.as_ref().map_or_else(String::new, |running| {
                let view = crate::bottom_bar::RunningView::new(running);
                view.detail.to_string()
            });
            let (state, aria) = match phase {
                JobPhase::Queued => ("Waiting".to_owned(), "Waiting".to_owned()),
                JobPhase::Planning => ("Planning…".to_owned(), "Planning".to_owned()),
                JobPhase::Running => (detail.clone(), "Running".to_owned()),
                JobPhase::PauseRequested => ("Pausing…".to_owned(), "Pausing".to_owned()),
                JobPhase::Paused => (detail, "Paused".to_owned()),
                JobPhase::AwaitingConflict => {
                    ("Waiting for your choice".to_owned(), "Waiting for your choice".to_owned())
                }
                JobPhase::Cancelling => ("Stopping…".to_owned(), "Stopping".to_owned()),
            };
            let label = crate::style::display_text(&operations::running_label(&label)).into_owned();
            div()
                .id(("activity-active", id.0))
                .debug_selector(move || format!("activity-active-{}", id.0))
                .role(Role::ListItem)
                .aria_label(format!("{label}, {aria}"))
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(label))
                        .child(div().text_size(px(11.)).text_color(muted).child(state)),
                )
                .when(phase == JobPhase::Running && pause_supported, |row| {
                    row.child(pill(("activity-pause-resume", id.0), "Pause", false, cx).on_click(cx.listener(
                        move |_, _, _, cx| {
                            if cx.global::<Operations>().active_id() == Some(id) {
                                operations::pause_active(cx);
                            }
                        },
                    )))
                })
                .when(matches!(phase, JobPhase::Paused | JobPhase::PauseRequested), |row| {
                    row.child(pill(("activity-pause-resume", id.0), "Resume", false, cx).on_click(cx.listener(
                        move |_, _, _, cx| {
                            if cx.global::<Operations>().active_id() == Some(id) {
                                operations::resume_active(cx);
                            }
                        },
                    )))
                })
                .when(has_conflict && prompt_job == Some(id), |row| {
                    row.child(pill(("activity-resolve", id.0), "Resolve", true, cx).on_click(cx.listener(
                        move |_, _, window, cx| {
                            if let Some(handle) = window.window_handle().downcast::<CrowWindow>() {
                                cx.defer(move |cx| {
                                    if cx.global::<Operations>().prompt.as_ref().is_some_and(|prompt| prompt.job == id)
                                    {
                                        operations::present_prompt_in(handle, cx);
                                    }
                                });
                            }
                        },
                    )))
                })
                .when(phase != JobPhase::Cancelling, |row| {
                    row.child(pill(("activity-cancel", id.0), "Cancel", false, cx).on_click(cx.listener(
                        move |_, _, _, cx| {
                            if cx.global::<Operations>().active_id() == Some(id) {
                                operations::cancel_active(cx);
                            }
                        },
                    )))
                })
        });
        let queued_rows: Vec<gpui::AnyElement> = queued
            .into_iter()
            .map(|(id, label)| {
                let label = crate::style::display_text(&label).into_owned();
                div()
                    .id(("activity-queued", id.0))
                    .debug_selector(move || format!("activity-queued-{}", id.0))
                    .role(Role::ListItem)
                    .aria_label(format!("{label}, waiting"))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(label))
                            .child(div().text_size(px(11.)).text_color(muted).child("Waiting")),
                    )
                    .child(
                        pill(("activity-cancel-queued", id.0), "Cancel", false, cx)
                            .on_click(cx.listener(move |_, _, _, cx| operations::cancel_queued(id, cx))),
                    )
                    .into_any_element()
            })
            .collect();
        let mode = cx.global::<Ui>().feedback.mode;
        let now = crate::clock::now();
        // A job that just finished leaves a picture of its row that shrinks and fades
        // (Full); Low and Off drop it at once.
        let still_running = running_label.is_some();
        if !still_running
            && mode == crow_core::motion::Effective::Full
            && let Some(label) = self.activity_last.take()
        {
            self.activity_ghost = Some((label, now));
        }
        self.activity_last = running_label;
        let ghost_row = self.activity_ghost.clone().and_then(|(label, started)| {
            let progress = crow_core::motion::progress(
                mode,
                started,
                now,
                crow_core::motion::ms::SECTION,
                crow_core::motion::ms::SECTION,
            );
            let Some(t) = progress.filter(|_| !still_running) else {
                self.activity_ghost = None;
                return None;
            };
            window.request_animation_frame();
            let left = 1.0 - t;
            let label = crate::style::display_text(&label).into_owned();
            Some(
                div()
                    .h(px(36. * left))
                    .overflow_hidden()
                    .opacity(left)
                    .debug_selector(|| "activity-ghost".into())
                    .child(div().h(px(36.)).flex().items_center().child(label))
                    .into_any_element(),
            )
        });
        let active_row = active_row.map(IntoElement::into_any_element).or(ghost_row);
        let trash_row = emptying_trash.then(|| {
            div()
                .id("activity-empty-trash")
                .debug_selector(|| "activity-empty-trash".into())
                .role(Role::ListItem)
                .aria_label("Emptying the Trash, running")
                .flex()
                .flex_col()
                .child(div().child("Empty the Trash"))
                .child(div().text_size(px(11.)).text_color(muted).child("Emptying…"))
                .into_any_element()
        });
        let has_history = !entries.is_empty();
        let history: Vec<_> = entries
            .into_iter()
            .map(|(i, label, when, undoable)| {
                div()
                    .id(("activity-entry", i))
                    .role(Role::ListItem)
                    .aria_label(format!("{label}, {when}"))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(label)
                            .child(div().text_size(px(11.)).text_color(muted).child(when)),
                    )
                    .when(undoable && idle, |row| {
                        row.child(
                            pill(("activity-undo", i as u64), "Undo", false, cx)
                                .on_click(cx.listener(move |this, _, _, cx| this.undo(Some(i), cx))),
                        )
                    })
            })
            .collect();
        if previous.values().any(|focus| focus.is_focused(window)) {
            cx.defer_in(window, |this, window, cx| {
                if this.activity_open(cx) && this.sheet.is_none() {
                    window.focus(&this.activity_focus, cx);
                }
            });
        }
        let forward_controls = controls.clone();
        div()
            .id("activity")
            .debug_selector(|| "activity".into())
            .track_focus(&self.activity_focus)
            .on_action(move |_: &SwitchFocus, window, cx| {
                cycle_focus(&forward_controls, true, window, cx);
                cx.stop_propagation();
            })
            .on_key_down(move |event, window, cx| {
                if event.keystroke.key == "tab" && event.keystroke.modifiers.shift {
                    cycle_focus(&controls, false, window, cx);
                    cx.stop_propagation();
                }
            })
            .on_action(cx.listener(|this, _: &crate::actions::CancelOperation, window, cx| {
                this.focus_center(window, cx);
                cx.stop_propagation();
            }))
            .role(Role::List)
            .aria_label("Activity")
            .flex()
            .flex_col()
            .gap_2()
            .when(active_row.is_none() && trash_row.is_none() && queued_rows.is_empty() && !has_history, |list| {
                list.child(div().text_color(muted).child("Nothing yet"))
            })
            .children(active_row)
            .children(trash_row)
            .children(queued_rows)
            .when(has_history, |list| list.child(div().h(px(1.)).bg(border).my_1()))
            .children(history)
    }
}

/// After Undo or Redo: a batch rename remaps selection and result rows to the
/// restored names first, so a saved or tag view keeps them while Spotlight catches
/// up; anything else just selects what came back and reruns the visible query.
fn finish_reversal(
    pane: &gpui::WeakEntity<Pane>,
    search: Option<Entity<SearchView>>,
    restored: Vec<PathBuf>,
    renamed: RenameMap,
    cx: &mut App,
) {
    if !renamed.is_empty()
        && let Some(search) = search
    {
        search.update(cx, |search, cx| search.apply_rename_map(&renamed, cx));
    }
    pane.update(cx, |pane, cx| {
        if renamed.is_empty() {
            pane.show_results(restored, false, cx);
            pane.refresh_saved(cx);
        } else {
            pane.apply_rename_map(&renamed, None, cx);
            pane.show_results(restored, false, cx);
        }
    })
    .ok();
}
