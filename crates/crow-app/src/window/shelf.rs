//! The shelf: a strip at the bottom of the sidebar that holds files while you move
//! between folders. It is opt-in (`[shelf] show = true` in sidebar.toml) and shows when
//! it holds something or a drag is in flight, so it is there to drop onto.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, CursorStyle, FontWeight, IntoElement, MouseButton, Role, SharedString, Window, div,
    prelude::*, px,
};

use super::{CrowWindow, DropLocation, Ui};
use crate::actions::{AddToShelf, ClearShelf, CopyShelfHere, MoveShelfHere};
use crate::drag_drop::DropHit;
use crate::operations;
use crate::pane::{DragHandoff, DraggedSelection, drag_combo, promote_drag};
use crate::style::color;

/// Rows shown before "+ n more".
const SHELF_ROWS: usize = 4;

impl CrowWindow {
    /// Whether the strip is on screen: switched on, and holding files or about to take a drop.
    pub(super) fn shelf_visible(&self, cx: &gpui::App) -> bool {
        let ui = cx.global::<Ui>();
        ui.sidebar.shelf.show && (!ui.shelf.is_empty() || cx.has_active_drag())
    }

    pub(super) fn render_shelf(&self, theme: &crow_theme::Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.shelf_visible(cx) {
            self.shelf_bounds.set(None);
            return None;
        }
        let (fg, muted, accent) = (theme.role("fg"), theme.role("fg-muted"), theme.role("accent"));
        let hover = self.drop_hover.as_ref().is_some_and(|hit| hit.location == DropLocation::Shelf);
        let items: Vec<_> = cx.global::<Ui>().shelf.items().to_vec();
        let count = items.len();
        let all_entries = Arc::new(items.iter().map(|item| (item.path.clone(), item.is_dir)).collect::<Vec<_>>());
        let rows = items.iter().take(SHELF_ROWS).enumerate().map(|(i, item)| {
            let path = item.path.clone();
            let name: SharedString = path
                .file_name()
                .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
                .into();
            let handoff = DragHandoff::new();
            let drag_icon = crate::style::icon(&path, item.is_dir, 22);
            let drag_label = name.clone();
            let entries = Arc::new(vec![(path.clone(), item.is_dir)]);
            let (reveal, remove) = (path.clone(), path.clone());
            div()
                .id(SharedString::from(format!("shelf-item-{i}")))
                .debug_selector(move || format!("shelf-item-{i}"))
                .role(Role::ListItem)
                .aria_label(name.to_string())
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py(px(2.))
                .rounded_md()
                .cursor(CursorStyle::OpenHand)
                .hover(|row| row.bg(color(theme.role("selection")).opacity(0.35)))
                .child(
                    div().flex_none().size(px(16.)).children(
                        crate::style::icon(&path, item.is_dir, 16).map(|image| gpui::img(image).size(px(16.))),
                    ),
                )
                .child(
                    div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(name.clone()),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("shelf-remove-{i}")))
                        .role(Role::Button)
                        .aria_label(format!("Take {name} off the shelf"))
                        .flex_none()
                        .px_1()
                        .text_color(color(muted))
                        .cursor_pointer()
                        .hover(|x| x.text_color(color(fg)))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let path = remove.clone();
                            crate::ui::edit_shelf(cx, |shelf| {
                                shelf.remove(&path);
                            });
                        }))
                        .child("×"),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let path = reveal.clone();
                    this.pane().update(cx, |pane, cx| pane.reveal(path, cx));
                }))
                .on_drag(DraggedSelection { entries }, {
                    let handoff = handoff.clone();
                    move |_value, offset, window, cx| {
                        drag_combo(drag_icon.clone(), &drag_label, offset, &handoff, window, cx)
                    }
                })
                .external_drag_payload::<DraggedSelection>(move |selection, window, cx| {
                    promote_drag(selection, &handoff, window, cx)
                })
        });
        let more = (count > SHELF_ROWS).then(|| {
            div().px_2().text_size(px(11.)).text_color(color(muted)).child(format!("+ {} more", count - SHELF_ROWS))
        });
        let _ = all_entries;
        let bounds = self.shelf_bounds.clone();
        let strip = div()
            .id("shelf")
            .debug_selector(|| "shelf".into())
            .role(Role::Group)
            .aria_label("Shelf")
            .mx(px(8.))
            .mb(px(8.))
            .p_1()
            .flex()
            .flex_col()
            .gap(px(1.))
            .rounded(px(10.))
            .border_1()
            .border_dashed()
            .border_color(color(if hover { accent } else { theme.role("border") }))
            .bg(color(theme.role("bg")))
            .text_size(px(13.))
            .text_color(color(fg))
            .child(
                div()
                    .flex()
                    .items_center()
                    .px_2()
                    .py(px(2.))
                    .gap_1()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Shelf")
                    .when(count > 0, |header| {
                        header.child(
                            div().text_color(color(muted)).font_weight(FontWeight::NORMAL).child(count.to_string()),
                        )
                    })
                    .when(count > 0, |header| {
                        header.child(
                            div()
                                .id("shelf-clear")
                                .role(Role::Button)
                                .aria_label("Clear shelf")
                                .ml_auto()
                                .text_size(px(11.))
                                .font_weight(FontWeight::NORMAL)
                                .text_color(color(muted))
                                .cursor_pointer()
                                .hover(|x| x.text_color(color(fg)))
                                .on_click(cx.listener(|this, _, window, cx| this.clear_shelf(&ClearShelf, window, cx)))
                                .child("Clear"),
                        )
                    }),
            )
            .children(rows)
            .children(more);
        Some(
            div()
                .on_children_prepainted(move |children, _, _| bounds.set(children.first().copied()))
                .child(strip)
                .into_any_element(),
        )
    }

    /// The shelf's drop hit: the strip, or the whole sidebar bottom while it can take a drop.
    pub(super) fn shelf_hit_at(&self, pointer: gpui::Point<gpui::Pixels>, cx: &gpui::App) -> Option<DropHit> {
        (self.shelf_visible(cx) && self.shelf_bounds.get().is_some_and(|bounds| bounds.contains(&pointer)))
            .then(DropHit::shelf)
    }

    /// Put dropped files on the shelf. A drop knows only paths, so a background pass
    /// learns which are folders before they are added.
    pub(super) fn drop_onto_shelf(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move {
            paths
                .into_iter()
                .map(|path| {
                    let is_dir = path.is_dir(); // io-ok: runs in background_spawn
                    (path, is_dir)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn_in(window, async move |_, cx| {
            let entries = task.await;
            cx.update(|_, cx| {
                crate::ui::edit_shelf(cx, |shelf| {
                    shelf.add(entries);
                })
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn add_to_shelf(&mut self, _: &AddToShelf, _: &mut Window, cx: &mut Context<Self>) {
        let entries: Vec<(PathBuf, bool)> = if let Some(search) = self.active_search(cx) {
            search.read(cx).selected_entry().into_iter().collect()
        } else {
            let pane = self.pane().read(cx);
            pane.selected_entries(cx).iter().map(|(path, is_dir)| (path.clone(), *is_dir)).collect()
        };
        if entries.is_empty() {
            return;
        }
        crate::ui::edit_shelf(cx, |shelf| {
            shelf.add(entries);
        });
    }

    pub(super) fn move_shelf_here(&mut self, _: &MoveShelfHere, window: &mut Window, cx: &mut Context<Self>) {
        self.shelf_here(false, window, cx);
    }

    pub(super) fn copy_shelf_here(&mut self, _: &CopyShelfHere, window: &mut Window, cx: &mut Context<Self>) {
        self.shelf_here(true, window, cx);
    }

    pub(super) fn clear_shelf(&mut self, _: &ClearShelf, _: &mut Window, cx: &mut Context<Self>) {
        crate::ui::edit_shelf(cx, |shelf| shelf.clear());
    }

    /// Move or copy everything on the shelf into the folder on screen. Moved files leave
    /// the shelf once the job is queued; copied files stay.
    fn shelf_here(&mut self, copy: bool, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.global::<Ui>().shelf.paths();
        if paths.is_empty() {
            return;
        }
        let into = self.pane().read(cx).current_dir(cx);
        let options = operations::DropOptions { copy, allow_move: true, allow_copy: true };
        let moved = paths.clone();
        let task = cx.background_spawn(async move { operations::drop_request(&paths, into, options) });
        cx.spawn_in(window, async move |this, cx| {
            let request = task.await;
            this.update_in(cx, |this, window, cx| {
                if let Some(request) = request {
                    if !copy {
                        crate::ui::edit_shelf(cx, |shelf| shelf.remove_all(&moved));
                    }
                    this.run_request(request, window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
