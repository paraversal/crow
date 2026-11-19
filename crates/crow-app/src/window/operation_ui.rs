//! File-operation UI: request planning, sheets, Activity and quit coordination.
//!
//! `Sheet` itself, `close_sheet`, onboarding and the shared focus surfaces stay
//! in `window.rs`; this module owns the operation content and dispatch. The
//! app-wide queue and its active job live in `operations.rs`.

use super::*;
use crate::actions::MoveToTrash;
use crate::operations::{self, JobPhase, Maintenance, Origin, RequestId, StopIntent};
use crow_core::motion::Fx;
use crow_fs::ops::{ConflictKey, Resolution};

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
    operations::cancel_all(cx);
    cx.spawn(async move |cx| {
        while cx.update(|cx| !cx.global::<Operations>().is_idle()) {
            cx.background_executor().timer(std::time::Duration::from_millis(30)).await;
        }
        cx.update(|cx| match intent {
            StopIntent::Quit => cx.quit(),
            StopIntent::Close => {
                if let Some(handle) = handle {
                    handle.update(cx, |_, window, _| window.remove_window()).ok();
                }
            }
        });
    })
    .detach();
}

impl CrowWindow {
    /// A red close or Cmd-W on the last tab: ask before cutting off pending work.
    pub(super) fn window_should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !operations::is_last_crow_window(window.window_handle().window_id(), cx) {
            return true;
        }
        if !cx.global::<Operations>().has_pending() {
            return true;
        }
        self.ask_to_stop(StopIntent::Close, window, cx);
        false
    }

    fn ask_to_stop(&mut self, intent: StopIntent, window: &mut Window, cx: &mut Context<Self>) {
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
        self.sheet = Some(Sheet::StopOperation { focus, intent });
        // A conflict sheet in another window must not stack with this question.
        cx.defer(|cx| operations::dismiss_prompt_sheets(None, cx));
        window.activate_window();
        cx.notify();
    }

    /// Whether `tab` is the active tab and still shows `dir`.
    pub(crate) fn origin_is_current(&self, tab: crow_core::TabId, dir: &Path, cx: &App) -> bool {
        let active = self.tabs.iter().nth(self.tabs.active_index()).map(|(id, _)| id);
        active == Some(tab) && self.tabs.iter().any(|(id, pane)| id == tab && pane.read(cx).current_dir(cx) == dir)
    }

    fn origin(&self, window: &Window, cx: &App) -> Option<Origin> {
        let pane = self.pane();
        let tab = self.tabs.iter().find(|(_, candidate)| *candidate == pane).map(|(tab, _)| tab)?;
        let window = window.window_handle().downcast::<CrowWindow>()?;
        Some(Origin { window, tab, dir: pane.read(cx).current_dir(cx) })
    }

    /// A permanent delete asks first; everything else enters the app-wide queue.
    pub(super) fn run_request(&mut self, request: Request, window: &mut Window, cx: &mut Context<Self>) {
        if let Request::DeletePermanently { paths } = request {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            self.sheet = Some(Sheet::ConfirmDelete { paths, focus });
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
                self.sheet = None;
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
        self.sheet = None;
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
        operations::undo(index, cx, move |restored, cx| {
            pane.update(cx, |pane, cx| {
                pane.show_results(restored, false, cx);
                pane.refresh_saved(cx);
            })
            .ok();
        });
    }

    pub(super) fn redo(&mut self, cx: &mut Context<Self>) {
        let pane = self.pane().downgrade();
        operations::redo(cx, move |results, cx| {
            pane.update(cx, |pane, cx| {
                pane.show_results(results, false, cx);
                pane.refresh_saved(cx);
            })
            .ok();
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
        cx.spawn_in(window, async move |this, cx| {
            let (folders, count) = cx
                .background_spawn(async move {
                    let folders = crate::trash::in_jail(crate::trash::folders(&home, &volumes), jail.as_deref());
                    let count = crow_fs::trash::count(&folders);
                    (folders, count)
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                if this.sheet.is_some() {
                    return;
                }
                if count == 0 {
                    operations::message("The Trash is already empty.", false, cx);
                    cx.notify();
                    return;
                }
                let focus = cx.focus_handle();
                window.focus(&focus, cx);
                this.sheet = Some(Sheet::ConfirmEmptyTrash { folders, focus });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Remove every folder's items in the background, then report and play the
    /// sound. Failures leave the rest removed and name what stayed.
    fn run_empty_trash(&mut self, folders: Vec<PathBuf>, cx: &mut Context<Self>) {
        if !operations::begin_maintenance(Maintenance::EmptyTrash, cx) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let report = cx.background_spawn(async move { crow_fs::trash::empty(&folders) }).await;
            let (text, is_error) = crate::trash::message(&report);
            let play = report.removed > 0;
            this.update(cx, |_, cx| {
                operations::message(text, is_error, cx);
                if play {
                    crate::feedback::play(crow_core::motion::Event::EmptyTrash, cx);
                }
                operations::end_maintenance(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn confirm_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.sheet, Some(Sheet::SavedQuery(_))) {
            self.confirm_saved_query_form(window, cx);
            return;
        }
        match self.sheet.take() {
            Some(Sheet::Conflict { job, key, choice, all, .. }) => {
                operations::resolve_prompt(job, key, choice, all, cx);
            }
            Some(Sheet::ConfirmDelete { paths, .. }) => {
                operations::enqueue(Request::DeletePermanently { paths }, self.origin(window, cx), cx);
            }
            Some(Sheet::ConfirmEmptyTrash { folders, .. }) => self.run_empty_trash(folders, cx),
            Some(Sheet::StopOperation { intent, .. }) => {
                let handle = window.window_handle().downcast::<CrowWindow>();
                stop_then(intent, handle, cx);
                return;
            }
            Some(Sheet::SavedQuery(_)) => {}
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

    pub(super) fn render_sheet(&self, window: &Window, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let sheet = self.sheet.as_ref()?;
        let theme = cx.global::<Ui>().theme.clone();
        let (fg, muted, accent) = (theme.role("fg"), theme.role("fg-muted"), theme.role("accent"));
        let trash_hint = match keys::text(&MoveToTrash, KeyScope::List, cx) {
            Some(keys) => format!("This can't be undone. Move to Trash ({keys}) can."),
            None => "This can't be undone. Move to Trash can.".to_owned(),
        };
        let button = |id: &'static str, label: String, primary: bool, focus: Option<&FocusHandle>| {
            let focused = focus.is_some_and(|focus| focus.is_focused(window));
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::Button)
                .aria_label(label.clone())
                .when_some(focus, |button, focus| button.track_focus(focus).tab_stop(true))
                .when(focused, |button| button.shadow(cursor_ring(&theme)))
                .px_3()
                .py_1()
                .rounded_md()
                .bg(color(if primary { accent } else { theme.role("surface-raised") }))
                .text_color(color(if primary { theme.role("on-accent") } else { fg }))
                .child(label)
        };
        let (title, body): (String, gpui::AnyElement) = match sheet {
            Sheet::ConfirmDelete { paths, .. } => {
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
                                    button("sheet-cancel", "Cancel".into(), false, None)
                                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                                )
                                .child(
                                    button("sheet-confirm", "Delete".into(), true, None)
                                        .on_click(cx.listener(|this, _, window, cx| this.confirm_sheet(window, cx))),
                                ),
                        )
                        .into_any_element(),
                )
            }
            Sheet::ConfirmEmptyTrash { .. } => (
                "Empty the Trash?".to_owned(),
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_color(color(muted)).child(
                        "Every item in the Trash, on this Mac and your other disks, will be deleted. This can't be undone.",
                    ))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                button("sheet-cancel", "Cancel".into(), false, None)
                                    .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                            )
                            .child(
                                button("sheet-confirm", "Empty Trash".into(), true, None)
                                    .on_click(cx.listener(|this, _, window, cx| this.confirm_sheet(window, cx))),
                            ),
                    )
                    .into_any_element(),
            ),
            Sheet::StopOperation { intent, .. } => {
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
                                    button("sheet-cancel", "Keep Running".into(), false, None)
                                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                                )
                                .child(
                                    button("sheet-confirm", confirm.into(), true, None)
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
            Sheet::Conflict { job, key, choice, all, .. } => {
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
                            "This item changed since the earlier answer was given. Choose again."
                        }
                    };
                    let choice = *choice;
                    let all = *all;
                    let option = |value: Resolution, label: &'static str, cx: &mut Context<Self>| {
                        let on = value == choice;
                        div()
                            .id(("conflict-choice", value as usize))
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
                                    .child(option(Resolution::KeepBoth, "Keep Both", cx))
                                    .child(option(Resolution::Replace, "Replace", cx))
                                    .child(option(Resolution::Skip, "Skip", cx)),
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
                                        button("sheet-cancel", "Decide Later".into(), false, None)
                                            .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                                    )
                                    .child(
                                        button("sheet-confirm", "Continue".into(), true, None)
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
        let onboarding_tab = cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
            if event.keystroke.key == "tab" {
                this.move_onboarding_focus(!event.keystroke.modifiers.shift, window, cx);
                cx.stop_propagation();
            }
        });
        Some(chrome::sheet(
            chrome::sheet_panel(
                "sheet",
                if matches!(sheet, Sheet::SavedQuery(_)) { SheetWidth::Wide } else { SheetWidth::Narrow },
                &theme,
            )
            .aria_label(title.clone())
            // keys::context_stack (KeyScope::Sheet) mirrors this.
            .key_context("Sheet")
            .when(!matches!(sheet, Sheet::SavedQuery(_)), |panel| panel.track_focus(sheet.focus()))
            .on_action(cx.listener(|this, _: &crate::actions::SheetConfirm, window, cx| this.confirm_sheet(window, cx)))
            .on_action(cx.listener(|this, _: &crate::actions::SheetCancel, window, cx| this.close_sheet(window, cx)))
            .when(matches!(sheet, Sheet::Onboarding { .. }), |panel| panel.on_key_down(onboarding_tab))
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
            cx.listener(|this, _, window, cx| this.close_sheet(window, cx)),
        ))
    }

    /// Active and queued jobs first, then recent terminal outcomes newest first.
    pub(super) fn render_activity(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let muted = color(theme.role("fg-muted"));
        let border = color(theme.role("border"));
        let (active, queued, running, prompt_job, idle) = {
            let ops = cx.global::<Operations>();
            let active = ops.active.as_ref().map(|active| {
                (active.id, active.label.clone(), active.phase, active.pause_supported, active.conflict.is_some())
            });
            let queued: Vec<(RequestId, String)> =
                ops.queue.iter().map(|queued| (queued.id, queued.label.clone())).collect();
            (active, queued, ops.running(), ops.prompt.as_ref().map(|prompt| prompt.job), ops.is_idle())
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
                (i, label, format!("{} {state}", when.format("%H:%M")), e.record.is_undoable() && !e.undone)
            })
            .collect();
        let pill = |id: (&'static str, u64), label: &'static str, primary: bool| {
            let debug = format!("{}-{}", id.0, id.1);
            div()
                .id(id)
                .debug_selector(move || debug.clone())
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
                JobPhase::Paused => (format!("Paused · {detail}"), "Paused".to_owned()),
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
                    row.child(
                        pill(("activity-pause", id.0), "Pause", false)
                            .on_click(cx.listener(|_, _, _, cx| operations::pause_active(cx))),
                    )
                })
                .when(matches!(phase, JobPhase::Paused | JobPhase::PauseRequested), |row| {
                    row.child(
                        pill(("activity-resume", id.0), "Resume", false)
                            .on_click(cx.listener(|_, _, _, cx| operations::resume_active(cx))),
                    )
                })
                .when(has_conflict && prompt_job == Some(id), |row| {
                    row.child(pill(("activity-resolve", id.0), "Resolve", true).on_click(cx.listener(
                        |this, _, window, cx| {
                            if let Some(handle) = window.window_handle().downcast::<CrowWindow>() {
                                operations::present_prompt_in(handle, cx);
                            }
                            this.show_activity = true;
                            cx.notify();
                        },
                    )))
                })
                .child(
                    pill(("activity-cancel", id.0), "Cancel", false)
                        .on_click(cx.listener(|_, _, _, cx| operations::cancel_active(cx))),
                )
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
                        pill(("activity-cancel-queued", id.0), "Cancel", false)
                            .on_click(cx.listener(move |_, _, _, cx| operations::cancel_queued(id, cx))),
                    )
                    .into_any_element()
            })
            .collect();
        let active_row = active_row.map(IntoElement::into_any_element);
        div()
            .id("activity")
            .role(Role::List)
            .aria_label("Activity")
            .absolute()
            .top(px(BAND_HEIGHT))
            .right(px(12.))
            .w(px(360.))
            .max_h(px(420.))
            .overflow_y_scroll()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .rounded_lg()
            .bg(color(theme.role("surface")))
            .border_1()
            .border_color(border)
            .shadow_lg()
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Activity"))
            .when(active_row.is_none() && queued_rows.is_empty() && entries.is_empty(), |list| {
                list.child(div().text_color(muted).child("Nothing yet"))
            })
            .children(active_row)
            .children(queued_rows)
            .when(!entries.is_empty(), |list| list.child(div().h(px(1.)).bg(border).my_1()))
            .children(entries.into_iter().map(|(i, label, when, undoable)| {
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
                            .flex()
                            .flex_col()
                            .child(label)
                            .child(div().text_size(px(11.)).text_color(muted).child(when)),
                    )
                    .when(undoable && idle, |row| {
                        row.child(
                            div()
                                .id(("activity-undo", i))
                                .role(Role::Button)
                                .aria_label("Undo")
                                .px_2()
                                .rounded_sm()
                                .bg(color(theme.role("surface-raised")))
                                .on_click(cx.listener(move |this, _, _, cx| this.undo(Some(i), cx)))
                                .child("Undo"),
                        )
                    })
            }))
    }
}
