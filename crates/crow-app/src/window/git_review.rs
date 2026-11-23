//! Read-only review of the changed paths in a folder or across pinned worktrees.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crow_plugin_git::StatusKind;
use gpui::{Context, FocusHandle, FontWeight, IntoElement, Role, ScrollHandle, Window, div, prelude::*, px};

use crate::actions::{ReviewGitChanges, SheetCancel, SheetConfirm};
use crate::chrome::{self, SheetWidth};
use crate::style::{active_selection, color, display_path};
use crate::ui::Ui;

use super::CrowWindow;

const LIMIT: usize = 240;
const ROW_HEIGHT: f32 = 26.;

pub(super) struct GitReview {
    pub(super) focus: FocusHandle,
    folder: PathBuf,
    pinned: bool,
    cursor: usize,
    selection: Option<(PathBuf, &'static str)>,
    scroll: ScrollHandle,
}

#[cfg(test)]
impl GitReview {
    pub(super) fn pinned(&self) -> bool {
        self.pinned
    }
}

#[derive(Clone)]
struct Change {
    path: PathBuf,
    display: String,
    kind: StatusKind,
    group: &'static str,
}

fn rows(folder: &Path, pinned: bool, cx: &mut Context<CrowWindow>) -> (Vec<Change>, &'static str) {
    let ui = cx.global::<Ui>();
    let folders = if pinned {
        ui.favorites().into_iter().flat_map(|(_, pins)| pins.into_iter().map(|pin| pin.path)).collect()
    } else {
        vec![folder.to_path_buf()]
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut found = false;
    let mut unavailable = false;
    for folder in folders {
        let Some((snapshot, relative)) = crate::git::snapshot_for_folder(cx, &folder) else { continue };
        if pinned && (!relative.as_os_str().is_empty() || !seen.insert(snapshot.identity.worktree.clone())) {
            continue;
        }
        found = true;
        unavailable |= snapshot.unavailable;
        for (source, destination) in snapshot.renames_by_source.range(relative.clone()..) {
            if !source.starts_with(&relative) {
                break;
            }
            if destination.starts_with(&relative) {
                continue;
            }
            out.push(Change {
                path: snapshot.identity.worktree.join(destination),
                display: format!("{} → {}", source.display(), destination.display()),
                kind: StatusKind::Renamed,
                group: "Staged",
            });
            if out.len() > LIMIT {
                break;
            }
        }
        if out.len() > LIMIT {
            break;
        }
        for (path, status) in snapshot.entries.range(relative.clone()..) {
            if !path.starts_with(&relative) {
                break;
            }
            if !status.is_changed() {
                continue;
            }
            let path = snapshot.identity.worktree.join(path);
            let display = if pinned {
                display_path(&path, &cx.global::<Ui>().home)
            } else {
                path.strip_prefix(snapshot.identity.worktree.join(&relative))
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned()
            };
            for (group, kind) in [
                ("Staged", status.index),
                ("Unstaged", status.worktree.filter(|kind| *kind != StatusKind::Untracked)),
                ("Untracked", status.worktree.filter(|kind| *kind == StatusKind::Untracked)),
            ] {
                if let Some(kind) = kind {
                    out.push(Change { path: path.clone(), display: display.clone(), kind, group });
                    if out.len() > LIMIT {
                        break;
                    }
                }
            }
            if out.len() > LIMIT {
                break;
            }
        }
        if out.len() > LIMIT {
            break;
        }
    }
    out.sort_by_key(|change| change.group);
    let message = if out.len() > LIMIT {
        out.truncate(LIMIT);
        "Showing the first 240 changes"
    } else if unavailable {
        "Git status is unavailable for some worktrees"
    } else if !found && pinned {
        "No pinned worktree status yet"
    } else if !found {
        "No Git worktree status yet"
    } else if out.is_empty() {
        "No changes"
    } else {
        ""
    };
    (out, message)
}

impl CrowWindow {
    pub(super) fn review_git_command(&mut self, _: &ReviewGitChanges, window: &mut Window, cx: &mut Context<Self>) {
        let Some(folder) = self.pane().read(cx).folder().map(Path::to_path_buf) else {
            crate::operations::message("Open a folder to review Git changes", false, cx);
            cx.notify();
            return;
        };
        self.open_git_review(folder, window, cx);
    }

    pub(super) fn open_git_review(&mut self, folder: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if !crate::git::provider_enabled(cx) || self.overlay_open() {
            return;
        }
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.git_review =
            Some(GitReview { focus, folder, pinned: false, cursor: 0, selection: None, scroll: ScrollHandle::new() });
        cx.notify();
    }

    fn close_git_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        retire!(self, cx, git_review);
        self.focus_center(window, cx);
        cx.notify();
    }

    fn reveal_git_change(&mut self, change: &Change, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(change.kind, StatusKind::Deleted) {
            return;
        }
        self.close_git_review(window, cx);
        if self.search.is_some() {
            self.close_search(window, cx);
        }
        self.pane().update(cx, |pane, cx| pane.reveal(change.path.clone(), cx));
    }

    pub(super) fn render_git_review(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        if self.git_review.is_some() && !crate::git::provider_enabled(cx) {
            // Git was switched off under the sheet: give focus back to the listing
            // instead of leaving it on a handle that no longer exists.
            let had_focus = self.git_review.as_ref().is_some_and(|review| review.focus.contains_focused(window, cx));
            retire!(self, cx, git_review);
            if had_focus {
                cx.defer_in(window, |this, window, cx| this.focus_center(window, cx));
            }
        }
        let review = self.git_review.as_ref()?;
        let (folder, pinned) = (review.folder.clone(), review.pinned);
        let (changes, message) = rows(&folder, pinned, cx);
        let review = self.git_review.as_mut().expect("review is open");
        if let Some((path, group)) = &review.selection
            && let Some(index) = changes.iter().position(|change| &change.path == path && change.group == *group)
        {
            review.cursor = index;
        }
        review.cursor = review.cursor.min(changes.len().saturating_sub(1));
        review.selection = changes.get(review.cursor).map(|change| (change.path.clone(), change.group));
        let (focus, scroll, cursor) = (review.focus.clone(), review.scroll.clone(), review.cursor);
        let selected = changes.get(cursor).cloned();
        let count = changes.len();
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let mode = ui.feedback.mode;
        let accent = color(theme.role("accent"));
        let muted = color(theme.role("fg-muted"));
        let title = "Review Git Changes";
        let scope = div()
            .id("git-review-scopes")
            .debug_selector(|| "git-review-scopes".into())
            .role(Role::TabList)
            .aria_label("Scope")
            .flex()
            .gap_2()
            .children([false, true].map(|all| {
                let label = if all { "Pinned Repositories" } else { "This Folder" };
                div()
                    .id(("git-review-scope", usize::from(all)))
                    .debug_selector(move || format!("git-review-scope-{}", usize::from(all)))
                    .role(Role::Tab)
                    .aria_label(label)
                    .aria_selected(all == pinned)
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .when(all == pinned, |tab| tab.bg(accent.opacity(0.15)))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(review) = &mut this.git_review {
                            review.pinned = all;
                            review.cursor = 0;
                            review.selection = None;
                            review.scroll.set_offset(gpui::point(px(0.), px(0.)));
                            cx.notify();
                        }
                    }))
            }));
        let rows = changes
            .into_iter()
            .enumerate()
            .map(|(index, change)| {
                let label = format!("{}, {}, {}", change.display, change.group, change.kind.label());
                div()
                    .id(("git-review-row", index))
                    .debug_selector(move || format!("git-review-row-{index}"))
                    .role(Role::ListBoxOption)
                    .aria_label(label)
                    .aria_selected(index == cursor)
                    .aria_position_in_set(index + 1)
                    .aria_size_of_set(count)
                    .when(index == cursor, |row| row.aria_active_descendant())
                    .flex()
                    .gap_2()
                    .items_center()
                    .h(px(ROW_HEIGHT))
                    .px_2()
                    .rounded_sm()
                    .when(index == cursor, |row| row.bg(active_selection(&theme)))
                    .child(div().w(px(65.)).text_color(muted).text_size(px(11.)).child(change.group))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(change.display.clone()),
                    )
                    .child(div().text_color(muted).child(change.kind.badge()))
                    .on_click(cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                        if let Some(review) = &mut this.git_review {
                            review.cursor = index;
                            review.selection = Some((change.path.clone(), change.group));
                            cx.notify();
                        }
                        if event.click_count() == 2 {
                            this.reveal_git_change(&change, window, cx);
                        }
                    }))
            })
            .collect::<Vec<_>>();
        Some(chrome::sheet(
            chrome::sheet_panel("git-review", SheetWidth::Wide, &theme)
                .aria_label(title)
                .key_context("Sheet")
                .track_focus(&focus)
                .on_action(cx.listener(|this, _: &SheetCancel, window, cx| this.close_git_review(window, cx)))
                .on_action(cx.listener(move |this, _: &SheetConfirm, window, cx| {
                    if let Some(change) = &selected {
                        this.reveal_git_change(change, window, cx);
                    }
                }))
                .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                    let Some(review) = &mut this.git_review else { return };
                    let key = event.keystroke.key.as_str();
                    match key {
                        "up" => review.cursor = review.cursor.saturating_sub(1),
                        "down" => review.cursor = (review.cursor + 1).min(count.saturating_sub(1)),
                        "tab" => {
                            review.pinned = !review.pinned;
                            review.cursor = 0;
                        }
                        _ => return,
                    }
                    review.selection = None;
                    review.scroll.scroll_to_item(review.cursor);
                    cx.stop_propagation();
                    cx.notify();
                }))
                .p_4()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                .child(scope)
                .when(!message.is_empty(), |panel| panel.child(div().text_color(muted).child(message)))
                .when(!rows.is_empty(), |panel| {
                    panel.child(
                        div()
                            .id("git-review-results")
                            .role(Role::ListBox)
                            .aria_label("Changed paths")
                            .max_h(px(240.))
                            .overflow_y_scroll()
                            .track_scroll(&scroll)
                            .children(rows),
                    )
                }),
            &theme,
            mode,
            self.exit_fade.get(),
            cx.listener(|this, _, window, cx| this.close_git_review(window, cx)),
        ))
    }
}

pub(crate) fn open_folder(path: PathBuf, window: &mut Window, cx: &mut gpui::App) {
    if let Some(root) = window.root::<CrowWindow>().flatten() {
        root.update(cx, |this, cx| this.open_git_review(path, window, cx));
    }
}
