//! File ▸ Save Search… and the sidebar's Edit Saved Search… sheet. The form
//! validates exactly what `saved-queries.toml` validates, so a record the form
//! writes always loads on the next launch.

use super::*;
use crate::search::SearchView;
use crow_core::{SavedExclusions, SavedKind, SavedMatch, SavedQuery, SavedQueryId, SavedScope};
use gpui::AnyElement;

pub(super) struct SavedQueryForm {
    pub(super) focus: FocusHandle,
    pub(super) id: Option<SavedQueryId>,
    pub(super) name: Entity<TextInput>,
    pub(super) root: Entity<TextInput>,
    pub(super) text: Entity<TextInput>,
    pub(super) kind: Entity<TextInput>,
    pub(super) days: Entity<TextInput>,
    pub(super) scope: SavedScope,
    pub(super) matching: SavedMatch,
    pub(super) include_hidden: bool,
    pub(super) enter_hidden_folders: bool,
    pub(super) exclusions: SavedExclusions,
    pub(super) error: Option<SharedString>,
}

fn field(
    value: String,
    placeholder: &'static str,
    aria: &'static str,
    cx: &mut Context<CrowWindow>,
) -> Entity<TextInput> {
    cx.new(|cx| TextInput::new(value, placeholder, cx).bare().with_aria_label(aria))
}

impl SavedQueryForm {
    pub(super) fn new(
        existing: Option<SavedQuery>,
        search: Option<Entity<SearchView>>,
        folder: Option<PathBuf>,
        cx: &mut Context<CrowWindow>,
    ) -> Self {
        let search_state = search.as_ref().map(|search| {
            let search = search.read(cx);
            (
                search.scope_value(),
                search.match_value(),
                search.draft_text(cx),
                search.hidden_flags(),
                search.exclusions_value(),
                search.predicate_values(),
            )
        });
        let (scope, matching, text, hidden, exclusions, predicates) = match &search_state {
            Some((scope, matching, text, hidden, exclusions, predicates)) => {
                (*scope, *matching, text.clone(), *hidden, *exclusions, predicates.clone())
            }
            None => existing.as_ref().map_or(
                (
                    SavedScope::Folder,
                    SavedMatch::All,
                    String::new(),
                    (true, false),
                    SavedExclusions::Configured,
                    (None, None),
                ),
                |saved| {
                    (
                        saved.scope,
                        saved.matching,
                        saved.text.clone(),
                        (saved.include_hidden, saved.enter_hidden_folders),
                        saved.exclusions,
                        (saved.kind.clone(), saved.modified_within_days),
                    )
                },
            ),
        };
        let name = existing.as_ref().map_or_else(
            || {
                let query = text.trim();
                if query.is_empty() { "Saved Search".to_owned() } else { query.to_owned() }
            },
            |saved| saved.name.clone(),
        );
        let root = existing
            .as_ref()
            .and_then(|saved| saved.root.clone())
            .or_else(|| (scope == SavedScope::Folder).then(|| folder.map(|folder| home_relative(&folder))).flatten());
        let (kind, days) = predicates;
        let focus = cx.focus_handle();
        let name = field(name, "Name", "Saved search name", cx);
        let root = field(root.unwrap_or_default(), "~/Documents", "Scope folder", cx);
        let text = field(text, "Text or empty with a predicate", "Search text", cx);
        let kind =
            field(kind.map(|kind| kind.key().to_owned()).unwrap_or_default(), "pdf, image, rs…", "Kind predicate", cx);
        let days = field(days.map(|days| days.to_string()).unwrap_or_default(), "30", "Modified within days", cx);
        Self {
            focus,
            id: existing.as_ref().map(|saved| saved.id.clone()),
            name,
            root,
            text,
            kind,
            days,
            scope,
            matching,
            include_hidden: hidden.0,
            enter_hidden_folders: hidden.1,
            exclusions,
            error: None,
        }
    }

    fn spec(&self, cx: &App) -> Result<SavedQuery, String> {
        let name = self.name.read(cx).text().trim().to_owned();
        if name.is_empty() {
            return Err("Give the saved search a name".into());
        }
        let root = if self.scope == SavedScope::Folder {
            let root = self.root.read(cx).text().trim().to_owned();
            if root.is_empty() {
                return Err("Choose the folder this search starts in".into());
            }
            Some(root)
        } else {
            None
        };
        let kind_text = self.kind.read(cx).text().trim().to_owned();
        let kind = if kind_text.is_empty() { None } else { Some(SavedKind::from_key(&kind_text)?) };
        let days_text = self.days.read(cx).text().trim().to_owned();
        let modified_within_days = if days_text.is_empty() {
            None
        } else {
            Some(
                days_text
                    .parse::<u32>()
                    .map_err(|_| "Modified within must be a positive whole number of days".to_owned())?,
            )
        };
        let id = match &self.id {
            Some(id) => id.clone(),
            None => SavedQuery::allocate_id(&name, &cx.global::<Ui>().saved_queries),
        };
        let query = SavedQuery {
            id,
            name,
            scope: self.scope,
            root,
            text: self.text.read(cx).text().trim().to_owned(),
            matching: self.matching,
            include_hidden: self.include_hidden,
            enter_hidden_folders: self.enter_hidden_folders,
            exclusions: self.exclusions,
            kind,
            modified_within_days,
        };
        query.validate(&cx.global::<Ui>().home)?;
        Ok(query)
    }
}

impl CrowWindow {
    pub(super) fn save_search(&mut self, _: &SaveSearch, window: &mut Window, cx: &mut Context<Self>) {
        let Some(search) = self.active_search(cx) else { return };
        let id = search.read(cx).saved_query_id();
        self.open_saved_query_form(id, Some(search), window, cx);
    }

    pub(super) fn edit_saved_search(&mut self, _: &EditSavedSearch, window: &mut Window, cx: &mut Context<Self>) {
        let id = self
            .active_search(cx)
            .and_then(|search| search.read(cx).saved_query_id())
            .or_else(|| self.sidebar.read(cx).cursor_saved_query(cx));
        if let Some(id) = id {
            self.open_saved_query_form(Some(id), None, window, cx);
        }
    }

    pub(super) fn remove_saved_search(&mut self, _: &RemoveSavedSearch, window: &mut Window, cx: &mut Context<Self>) {
        let id = self
            .active_search(cx)
            .and_then(|search| search.read(cx).saved_query_id())
            .or_else(|| self.sidebar.read(cx).cursor_saved_query(cx));
        if let Some(id) = id {
            ui::remove_saved_query(id, None, cx);
            self.focus_center(window, cx);
            cx.notify();
        }
    }

    pub(super) fn edit_saved_search_by_id(&mut self, id: SavedQueryId, window: &mut Window, cx: &mut Context<Self>) {
        self.open_saved_query_form(Some(id), None, window, cx);
    }

    fn open_saved_query_form(
        &mut self,
        id: Option<SavedQueryId>,
        search: Option<Entity<SearchView>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sheet.is_some() {
            return;
        }
        let existing = id.as_ref().and_then(|id| cx.global::<Ui>().saved_queries.get(id).cloned());
        let folder = self.pane().read(cx).folder().map(Path::to_path_buf);
        let form = SavedQueryForm::new(existing, search, folder, cx);
        let focus = form.name.read(cx).focus_handle(cx);
        self.sheet = Some(Sheet::SavedQuery(Box::new(form)));
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(super) fn confirm_saved_query_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Sheet::SavedQuery(form)) = &self.sheet else { return };
        match form.spec(cx) {
            Ok(query) => {
                ui::save_saved_query(query, None, cx);
                self.sheet = None;
                self.focus_center(window, cx);
                cx.notify();
            }
            Err(error) => {
                if let Some(Sheet::SavedQuery(form)) = &mut self.sheet {
                    form.error = Some(error.into());
                }
                cx.notify();
            }
        }
    }

    pub(super) fn remove_saved_query_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Sheet::SavedQuery(form)) = &self.sheet else { return };
        let Some(id) = form.id.clone() else { return };
        ui::remove_saved_query(id, None, cx);
        self.sheet = None;
        self.focus_center(window, cx);
        cx.notify();
    }

    pub(super) fn set_saved_form_scope(&mut self, scope: SavedScope, cx: &mut Context<Self>) {
        if let Some(Sheet::SavedQuery(form)) = &mut self.sheet {
            form.scope = scope;
            cx.notify();
        }
    }

    pub(super) fn set_saved_form_match(&mut self, matching: SavedMatch, cx: &mut Context<Self>) {
        if let Some(Sheet::SavedQuery(form)) = &mut self.sheet {
            form.matching = matching;
            cx.notify();
        }
    }

    pub(super) fn toggle_saved_form_hidden(&mut self, which: u8, cx: &mut Context<Self>) {
        if let Some(Sheet::SavedQuery(form)) = &mut self.sheet {
            if which == 0 {
                form.include_hidden = !form.include_hidden;
            } else {
                form.enter_hidden_folders = !form.enter_hidden_folders;
            }
            cx.notify();
        }
    }

    pub(super) fn set_saved_form_exclusions(&mut self, exclusions: SavedExclusions, cx: &mut Context<Self>) {
        if let Some(Sheet::SavedQuery(form)) = &mut self.sheet {
            form.exclusions = exclusions;
            cx.notify();
        }
    }

    pub(super) fn render_saved_query_form(&self, form: &SavedQueryForm, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Ui>().theme.clone();
        let muted = color(theme.role("fg-muted"));
        let label = |text: &'static str| div().w(px(110.)).flex_none().text_color(muted).child(text);
        let row = |label_text: &'static str, input: Entity<TextInput>| {
            div().flex().items_center().gap_3().child(label(label_text)).child(div().flex_1().child(input))
        };
        let choice = |id: &'static str, text: &'static str, selected: bool| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::RadioButton)
                .aria_label(text)
                .aria_selected(selected)
                .px_2()
                .py_1()
                .rounded_sm()
                .text_size(px(12.))
                .when(selected, |choice| {
                    choice.bg(color(theme.role("surface-raised"))).text_color(color(theme.role("fg")))
                })
                .child(text)
        };
        let toggle = |id: &'static str, text: &'static str, on: bool| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::CheckBox)
                .aria_label(text)
                .aria_selected(on)
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(12.))
                .text_color(if on { color(theme.role("fg")) } else { muted })
                .child(if on { "☑" } else { "☐" })
                .child(text)
        };
        let button = |id: &'static str, text: &'static str, primary: bool| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::Button)
                .aria_label(text)
                .px_3()
                .py_1()
                .rounded_md()
                .bg(color(if primary { theme.role("accent") } else { theme.role("surface-raised") }))
                .text_color(color(if primary { theme.role("on-accent") } else { theme.role("fg") }))
                .child(text)
        };
        div()
            .id("saved-query-form")
            .debug_selector(|| "saved-query-form".into())
            .flex()
            .flex_col()
            .gap_2()
            .child(row("Name", form.name.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(label("Scope"))
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(choice("saved-scope-folder", "Folder", form.scope == SavedScope::Folder).on_click(
                                cx.listener(|this, _, _, cx| this.set_saved_form_scope(SavedScope::Folder, cx)),
                            ))
                            .child(choice("saved-scope-home", "Home", form.scope == SavedScope::Home).on_click(
                                cx.listener(|this, _, _, cx| this.set_saved_form_scope(SavedScope::Home, cx)),
                            ))
                            .child(choice("saved-scope-this-mac", "This Mac", form.scope == SavedScope::ThisMac).on_click(
                                cx.listener(|this, _, _, cx| this.set_saved_form_scope(SavedScope::ThisMac, cx)),
                            )),
                    ),
            )
            .when(form.scope == SavedScope::Folder, |form_row| form_row.child(row("Folder", form.root.clone())))
            .child(row("Text", form.text.clone()))
            .child(row("Kind", form.kind.clone()))
            .child(row("Modified", form.days.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(label("Match"))
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(choice("saved-match-all", "All", form.matching == SavedMatch::All).on_click(
                                cx.listener(|this, _, _, cx| this.set_saved_form_match(SavedMatch::All, cx)),
                            ))
                            .child(choice("saved-match-names", "Names", form.matching == SavedMatch::Names).on_click(
                                cx.listener(|this, _, _, cx| this.set_saved_form_match(SavedMatch::Names, cx)),
                            ))
                            .child(choice("saved-match-contents", "Contents", form.matching == SavedMatch::Contents).on_click(
                                cx.listener(|this, _, _, cx| this.set_saved_form_match(SavedMatch::Contents, cx)),
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(label("Hidden"))
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(
                                toggle("saved-hidden-include", "Include hidden names", form.include_hidden)
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_saved_form_hidden(0, cx))),
                            )
                            .child(
                                toggle("saved-hidden-enter", "Enter hidden folders", form.enter_hidden_folders)
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_saved_form_hidden(1, cx))),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(label("Exclusions"))
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                choice("saved-exclusions-configured", "exclude.toml", form.exclusions == SavedExclusions::Configured)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_saved_form_exclusions(SavedExclusions::Configured, cx)
                                    })),
                            )
                            .child(
                                choice("saved-exclusions-everything", "Everything", form.exclusions == SavedExclusions::Everything)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_saved_form_exclusions(SavedExclusions::Everything, cx)
                                    })),
                            ),
                    ),
            )
            .when_some(form.error.clone(), |form, error| {
                form.child(div().text_size(px(12.)).text_color(color(theme.role("error"))).child(error))
            })
            .child(div().text_size(px(11.)).text_color(muted).child(
                "An empty text is allowed with a kind or a modification age. This Mac searches names through Spotlight.",
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pt_1()
                    .when(form.id.is_some(), |buttons| {
                        buttons.child(
                            button("saved-remove", "Remove", false)
                                .on_click(cx.listener(|this, _, window, cx| this.remove_saved_query_form(window, cx))),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        button("saved-cancel", "Cancel", false)
                            .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                    )
                    .child(
                        button("saved-save", "Save", true)
                            .on_click(cx.listener(|this, _, window, cx| this.confirm_saved_query_form(window, cx))),
                    ),
            )
            .into_any_element()
    }
}
