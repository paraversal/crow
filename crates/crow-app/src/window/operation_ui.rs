//! File-operation UI: request planning, sheets, Activity and quit coordination.
//!
//! `Sheet` itself, `close_sheet`, onboarding and the shared focus surfaces stay
//! in `window.rs`; this module owns the operation content and dispatch. One
//! operation coordinator remains app-wide in `operations.rs` pending P5B.

use super::*;
use crate::actions::MoveToTrash;
use crow_core::motion::Fx;

/// Quit, unless an operation is running: then the active Crow window asks first.
/// With no window open, one opens at the launch folder to ask.
pub(crate) fn quit(cx: &mut App) {
    if cx.global::<Operations>().running.is_none() {
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
            if handle.update(cx, |this, window, cx| this.ask_to_stop(window, cx)).is_err() {
                cx.quit();
            }
        }),
        None => match open(launch_folder(cx), false, None, cx) {
            Some(handle) => cx.defer(move |cx| {
                if handle.update(cx, |this, window, cx| this.ask_to_stop(window, cx)).is_err() {
                    stop_then_quit(cx);
                }
            }),
            None => stop_then_quit(cx),
        },
    }
}

/// Cancel the running operation, wait for it to wind down (its undo record lands in
/// the log), then quit.
fn stop_then_quit(cx: &mut App) {
    operations::cancel_running(cx);
    cx.spawn(async move |cx| {
        while cx.update(|cx| cx.global::<Operations>().running.is_some()) {
            cx.background_executor().timer(std::time::Duration::from_millis(30)).await;
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

impl CrowWindow {
    fn ask_to_stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.sheet, Some(Sheet::StopOperation { .. })) {
            return;
        }
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.sheet = Some(Sheet::StopOperation { focus });
        window.activate_window();
        cx.notify();
    }

    /// Plan in the background; ask first when there are conflicts or the change is
    /// permanent; otherwise run it.
    pub(super) fn run_request(&mut self, request: Request, window: &mut Window, cx: &mut Context<Self>) {
        if let Request::DeletePermanently { paths } = request {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            self.sheet = Some(Sheet::ConfirmDelete { paths, focus });
            cx.notify();
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let planned = cx.background_spawn(async move { crow_fs::ops::plan(request) }).await;
            this.update_in(cx, |this, window, cx| match planned {
                Err(e) => {
                    operations::message(e.to_string(), true, cx);
                    cx.notify();
                }
                Ok(plan) if plan.items.is_empty() => {}
                Ok(plan) if plan.conflicts().next().is_some() => {
                    let focus = cx.focus_handle();
                    window.focus(&focus, cx);
                    let choices = vec![Resolution::KeepBoth; plan.conflicts().count()];
                    this.sheet = Some(Sheet::Conflicts { plan, choices, focus });
                    cx.notify();
                }
                Ok(plan) => this.start(plan, HashMap::new(), cx),
            })
            .ok();
        })
        .detach();
    }

    fn start(&mut self, plan: Plan, resolutions: HashMap<PathBuf, Resolution>, cx: &mut Context<Self>) {
        let rename_result = matches!(plan.request, Request::NewFolder { .. } | Request::NewFile { .. });
        if rename_result {
            // The new item must be visible: a filter that hides it would hide the
            // rename field that follows (findings #10).
            self.pane().update(cx, |pane, cx| pane.clear_filter_for_new_item(cx));
        }
        let trash_paths = match &plan.request {
            Request::Trash { paths } => Some(&paths[..]),
            _ => None,
        };
        let move_paths = match &plan.request {
            Request::Move { paths, into } => Some((&paths[..], into.as_path())),
            _ => None,
        };
        // Only the error callback needs an owned copy; the effects borrow the plan.
        let failed_paths = match &plan.request {
            Request::Trash { paths }
            | Request::Move { paths, .. }
            | Request::Copy { paths, .. }
            | Request::Duplicate { paths } => Some(paths.clone()),
            _ => None,
        };
        let pane = self.pane().downgrade();
        // The effect starts with the operation, not the key press: a conflict or a
        // delete sheet may have come first, and a second operation is refused.
        if cx.global::<Operations>().running.is_none() {
            if let Some(paths) = trash_paths {
                pane.update(cx, |pane, cx| pane.start_effect(paths, Fx::Trash, cx)).ok();
            }
            if let Some((paths, into)) = move_paths {
                pane.update(cx, |pane, cx| pane.start_effect(paths, Fx::MoveOut, cx)).ok();
                // The folder the items land in pulses, in the list and in the sidebar.
                let into = into.to_path_buf();
                pane.update(cx, |pane, cx| pane.start_effect(std::slice::from_ref(&into), Fx::Receive, cx)).ok();
                self.sidebar.update(cx, |sidebar, cx| sidebar.pulse(into, cx));
            }
        }
        operations::execute(plan, resolutions, cx, move |record, error, cx| {
            if error.is_some()
                && let Some(paths) = &failed_paths
            {
                // Whatever is still there refuses.
                pane.update(cx, |pane, cx| pane.start_effect(paths, Fx::Refuse, cx)).ok();
            }
            let results = record.results();
            pane.update(cx, |pane, cx| {
                pane.show_results(results, rename_result, cx);
                pane.refresh_saved(cx);
            })
            .ok();
        });
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
        cx.spawn(async move |this, cx| {
            let report = cx.background_spawn(async move { crow_fs::trash::empty(&folders) }).await;
            let (text, is_error) = crate::trash::message(&report);
            let play = report.removed > 0;
            this.update(cx, |_, cx| {
                operations::message(text, is_error, cx);
                if play {
                    crate::feedback::play(crow_core::motion::Event::EmptyTrash, cx);
                }
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
            Some(Sheet::Conflicts { plan, choices, .. }) => {
                let targets: Vec<PathBuf> = plan.conflicts().map(|i| i.target.clone()).collect();
                let resolutions = targets.into_iter().zip(choices).collect();
                self.start(plan, resolutions, cx);
            }
            Some(Sheet::ConfirmDelete { paths, .. }) => {
                match crow_fs::ops::plan(Request::DeletePermanently { paths }) {
                    Ok(plan) => self.start(plan, HashMap::new(), cx),
                    Err(e) => operations::message(e.to_string(), true, cx),
                }
            }
            Some(Sheet::ConfirmEmptyTrash { folders, .. }) => self.run_empty_trash(folders, cx),
            Some(Sheet::StopOperation { .. }) => {
                stop_then_quit(cx);
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

    fn set_choice(&mut self, index: Option<usize>, choice: Resolution, cx: &mut Context<Self>) {
        if let Some(Sheet::Conflicts { choices, .. }) = &mut self.sheet {
            match index {
                Some(i) => {
                    if let Some(c) = choices.get_mut(i) {
                        *c = choice;
                    }
                }
                None => choices.iter_mut().for_each(|c| *c = choice),
            }
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
            Sheet::StopOperation { .. } => {
                let label = cx
                    .global::<Operations>()
                    .running
                    .as_ref()
                    .map_or_else(|| "The operation".to_owned(), |r| operations::running_label(&r.label));
                let (verb, confirm) = ("Quitting", "Stop and Quit");
                (
                    format!("{label} is still running"),
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(div().text_color(color(muted)).child(format!(
                            "{verb} would cut it off. Stop it cleanly first? Items already finished stay (Undo can reverse them); the one in progress is left out, never half done."
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
            Sheet::Conflicts { plan, choices, .. } => {
                let into = plan
                    .items
                    .first()
                    .and_then(|i| i.target.parent())
                    .map(crate::style::display_name)
                    .unwrap_or_default();
                let rows: Vec<(usize, String, Resolution)> = plan
                    .conflicts()
                    .zip(choices.iter().copied())
                    .enumerate()
                    .map(|(i, (item, choice))| {
                        (
                            i,
                            item.target.file_name().map_or_else(String::new, |name| {
                                crate::style::display_text(&name.to_string_lossy()).into_owned()
                            }),
                            choice,
                        )
                    })
                    .collect();
                let option = |row: usize,
                              label: &'static str,
                              value: Resolution,
                              current: Resolution,
                              cx: &mut Context<Self>| {
                    let on = value == current;
                    div()
                        .id(("choice", row * 3 + value as usize))
                        .role(Role::RadioButton)
                        .aria_label(label)
                        .aria_selected(on)
                        .px_2()
                        .rounded_sm()
                        .text_size(px(12.))
                        .when(on, |o| o.bg(active_selection(&theme)).text_color(color(fg)))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_choice(Some(row), value, cx)))
                        .child(label)
                };
                let n = rows.len();
                (
                    if n == 1 {
                        format!("An item named “{}” already exists in “{into}”", rows[0].1)
                    } else {
                        format!("{n} items already exist in “{into}”")
                    },
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .children(rows.into_iter().map(|(i, name, choice)| {
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().flex_1().overflow_hidden().whitespace_nowrap().child(name))
                                .child(option(i, "Keep Both", Resolution::KeepBoth, choice, cx))
                                .child(option(i, "Replace", Resolution::Replace, choice, cx))
                                .child(option(i, "Skip", Resolution::Skip, choice, cx))
                        }))
                        .child(div().text_size(px(12.)).text_color(color(muted)).child(
                            "Keep Both adds a number to the new name. Replace moves the existing item to the Trash.",
                        ))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .pt_2()
                                .child(button("all-keep", "Keep Both for All".into(), false, None).on_click(
                                    cx.listener(|this, _, _, cx| this.set_choice(None, Resolution::KeepBoth, cx)),
                                ))
                                .child(button("all-skip", "Skip All".into(), false, None).on_click(
                                    cx.listener(|this, _, _, cx| this.set_choice(None, Resolution::Skip, cx)),
                                ))
                                .child(div().flex_1())
                                .child(
                                    button("sheet-cancel", "Cancel".into(), false, None)
                                        .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                                )
                                .child(
                                    button("sheet-confirm", "Continue".into(), true, None)
                                        .on_click(cx.listener(|this, _, window, cx| this.confirm_sheet(window, cx))),
                                ),
                        )
                        .into_any_element(),
                )
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

    /// Recent operations, newest first, each with its own Undo.
    pub(super) fn render_activity(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Ui>().theme.clone();
        let muted = color(theme.role("fg-muted"));
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
            .border_color(color(theme.role("border")))
            .shadow_lg()
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Activity"))
            .when(entries.is_empty(), |list| list.child(div().text_color(muted).child("Nothing yet")))
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
                    .when(undoable, |row| {
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
