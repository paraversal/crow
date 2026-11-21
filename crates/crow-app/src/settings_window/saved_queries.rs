use super::*;
use crate::chrome::{self, SheetWidth};
use crate::window::saved_query_form::{FormChange, SavedQueryForm};
use crow_core::SavedQueryId;

impl SettingsWindow {
    pub(super) fn selected_saved_query<'a>(&'a self, cx: &'a App) -> Option<&'a SavedQueryId> {
        let queries = &cx.global::<Ui>().saved_queries;
        self.saved_query_selection
            .as_ref()
            .filter(|id| queries.contains(id))
            .or_else(|| queries.queries.first().map(|query| &query.id))
    }

    pub(super) fn move_saved_query_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(selected) = self.selected_saved_query(cx) else { return };
        let queries = &cx.global::<Ui>().saved_queries.queries;
        let current = queries.iter().position(|query| &query.id == selected).expect("selected query");
        let next = current.saturating_add_signed(delta).min(queries.len() - 1);
        self.saved_query_selection = Some(queries[next].id.clone());
        self.saved_query_scroll.scroll_to_item(next);
        cx.notify();
    }

    pub(super) fn open_saved_query_editor(
        &mut self,
        id: Option<SavedQueryId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ui = cx.global::<Ui>();
        if self.saved_query_form.is_some() || ui.file_error("saved-queries.toml").is_some() {
            return;
        }
        let existing = id.as_ref().and_then(|id| ui.saved_queries.get(id)).cloned();
        if id.is_some() && existing.is_none() {
            return;
        }
        let folder = ui.settings_folder.clone().or_else(|| Some(ui.home.clone()));
        let mut form = SavedQueryForm::new(existing, None, folder, cx);
        form.subscribe(Self::saved_query_form_action, window, cx);
        let focus = form.name.read(cx).focus_handle(cx);
        self.saved_query_form = Some(Box::new(form));
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(super) fn close_saved_query_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.saved_query_form.take() else { return };
        let control = if form.id.is_some() && ControlId::SavedQueryList.enabled(cx) {
            ControlId::SavedQueryList
        } else {
            ControlId::NewSavedQuery
        };
        let focus = if control.enabled(cx) { self.handle(control) } else { self.search_handle(cx) };
        window.focus(&focus, cx);
        self.scroll_to_focused(window);
        cx.notify();
    }

    fn saved_query_form_action(&mut self, action: FormChange, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &mut self.saved_query_form else { return };
        let action = form.apply(action, window, cx);
        if action == FormChange::Cancel {
            self.close_saved_query_editor(window, cx);
            return;
        }
        if form.writing {
            return;
        }
        let weak = cx.weak_entity();
        let handle = window.window_handle();
        let form_focus = form.focus.clone();
        let completion: ui::SavedQueryWriteCompletion = Box::new(move |result, cx| {
            handle
                .update(cx, |_, window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        let Some(form) = &mut this.saved_query_form else { return };
                        if form.focus != form_focus {
                            return;
                        }
                        match result {
                            Ok(()) => this.close_saved_query_editor(window, cx),
                            Err(error) => {
                                form.writing = false;
                                form.error = Some(error.into());
                                cx.notify();
                            }
                        }
                    });
                })
                .ok();
        });
        match action {
            FormChange::Save => match form.spec(cx) {
                Ok(query) => {
                    form.writing = true;
                    form.error = None;
                    ui::save_saved_query(query, Some(completion), cx);
                }
                Err(error) => form.error = Some(error.into()),
            },
            FormChange::Remove => {
                if let Some(id) = &form.id {
                    form.writing = true;
                    form.error = None;
                    ui::remove_saved_query(id.clone(), Some(completion), cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    pub(super) fn render_saved_queries(
        &self,
        window: &Window,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let add = ControlId::NewSavedQuery;
        let list = ControlId::SavedQueryList;
        let create = callback(cx, |this, window, cx| this.open_saved_query_editor(None, window, cx));
        let records = &cx.global::<Ui>().saved_queries.queries;
        let selected = self.selected_saved_query(cx);
        let focused = self.is_focused(list, window);
        let enabled = list.enabled(cx);
        let rows: Vec<_> = records
            .iter()
            .enumerate()
            .map(|(index, query)| {
                let id = query.id.clone();
                let current = selected == Some(&id);
                let scope = query.root.clone().unwrap_or_else(|| match query.scope {
                    crow_core::SavedScope::ThisMac => "This Mac".into(),
                    _ => "Home".into(),
                });
                div()
                    .id(SharedString::from(format!("settings-saved-query-{}", id.as_str())))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("settings-saved-query-{}", id.as_str())
                    })
                    .role(Role::ListBoxOption)
                    .aria_label(query.name.clone())
                    .aria_description(scope.clone())
                    .aria_selected(current)
                    .aria_position_in_set(index + 1)
                    .aria_size_of_set(records.len())
                    .when(current && focused, |row| row.aria_active_descendant().shadow(cursor_ring(theme)))
                    .rounded_md()
                    .px_2()
                    .py_1()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .when(enabled, |row| {
                        row.cursor_pointer().hover(|row| row.bg(color(theme.role("surface")))).on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.saved_query_selection = Some(id.clone());
                                this.open_saved_query_editor(Some(id.clone()), window, cx);
                            },
                        ))
                    })
                    .child(div().min_w_0().truncate().child(query.name.clone()))
                    .child(div().min_w_0().truncate().text_color(color(theme.role("fg-muted"))).child(scope))
            })
            .collect();
        div()
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
                            .id("settings-saved-query-heading")
                            .role(Role::Heading)
                            .aria_level(3)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(list.label()),
                    )
                    .child(controls::link_button(
                        add.key(),
                        add.label(),
                        add.enabled(cx),
                        self.style(add, window, theme),
                        create,
                    )),
            )
            .when(!rows.is_empty(), |unit| {
                unit.child(
                    div()
                        .id(list.key())
                        .debug_selector(move || list.key().to_owned())
                        .role(Role::ListBox)
                        .aria_label(list.label())
                        .track_focus(&self.handle(list))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .max_h(px(200.))
                        .overflow_y_scroll()
                        .track_scroll(&self.saved_query_scroll)
                        .children(rows),
                )
            })
            .when(records.is_empty(), |unit| {
                unit.child(div().text_color(color(theme.role("fg-muted"))).child("No saved queries"))
            })
            .into_any_element()
    }

    pub(super) fn render_saved_query_editor(
        &self,
        window: &Window,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let form = self.saved_query_form.as_ref()?;
        let title = if form.id.is_some() { "Edit Saved Query" } else { "New Saved Query" };
        let content = form.render(window, cx, Self::saved_query_form_action);
        Some(
            chrome::sheet(
                chrome::sheet_panel("settings-saved-query-editor", SheetWidth::Wide, theme)
                    .aria_label(title)
                    // The sheet context blocks ⌘W and the other window-level keys behind it.
                    .key_context("Sheet")
                    .max_w(window.viewport_size().width - px(24.))
                    .max_h(window.viewport_size().height - px(50.))
                    .overflow_y_scroll()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(content),
                theme,
                cx.global::<Ui>().feedback.mode,
                None,
                cx.listener(|this, _, window, cx| this.close_saved_query_editor(window, cx)),
            )
            .into_any_element(),
        )
    }
}
