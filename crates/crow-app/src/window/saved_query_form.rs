//! File ▸ Save Search… and the sidebar's Edit Saved Search… sheet. The form
//! validates exactly what `saved-queries.toml` validates, so a record the form
//! writes always loads on the next launch.

use super::*;
use crate::search::SearchView;
use crow_core::{SavedExclusions, SavedKind, SavedMatch, SavedQuery, SavedQueryId, SavedScope};
use gpui::AnyElement;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormChange {
    Scope(SavedScope),
    Match(SavedMatch),
    IncludeHidden,
    EnterHiddenFolders,
    Exclusions(SavedExclusions),
    Save,
    Remove,
    Cancel,
    Next(isize),
    Activate,
}

pub(crate) struct SavedQueryForm {
    pub(crate) focus: FocusHandle,
    pub(crate) id: Option<SavedQueryId>,
    pub(crate) name: Entity<TextInput>,
    pub(crate) root: Entity<TextInput>,
    pub(crate) text: Entity<TextInput>,
    pub(crate) kind: Entity<TextInput>,
    pub(crate) days: Entity<TextInput>,
    pub(crate) tags: Entity<TextInput>,
    pub(crate) scope: SavedScope,
    pub(crate) matching: SavedMatch,
    pub(crate) include_hidden: bool,
    pub(crate) enter_hidden_folders: bool,
    pub(crate) exclusions: SavedExclusions,
    pub(crate) error: Option<SharedString>,
    pub(crate) writing: bool,
    controls: [(FormChange, FocusHandle); 13],
    subscriptions: Vec<Subscription>,
}

/// A tag field that stays comma-separated for ordinary names; a name containing
/// a comma, or starting with `[` (which would read as the JSON form), is shown
/// as a JSON string array so editing it cannot split or reinterpret the value.
fn tag_field_text(tags: &[String]) -> String {
    if tags.iter().any(|tag| tag.contains(',') || tag.starts_with('[')) {
        serde_json::to_string(tags).unwrap_or_default()
    } else {
        tags.join(", ")
    }
}

/// The inverse of [`tag_field_text`]: a JSON string array when the text is one,
/// otherwise a comma-separated list, so `[Urgent], Work` is two tags and not a
/// failed array.
fn parse_tags_field(text: &str) -> Result<Vec<String>, String> {
    if text.starts_with('[')
        && let Ok(values) = serde_json::from_str::<Vec<String>>(text)
    {
        return crow_core::normalize_tags(values);
    }
    crow_core::normalize_tags(text.split(',').map(str::trim).filter(|tag| !tag.is_empty()))
}

fn field<T: 'static>(
    value: String,
    placeholder: &'static str,
    aria: &'static str,
    cx: &mut Context<T>,
) -> Entity<TextInput> {
    cx.new(|cx| TextInput::new(value, placeholder, cx).bare().with_aria_label(aria))
}

impl SavedQueryForm {
    pub(crate) fn new<T: 'static>(
        existing: Option<SavedQuery>,
        search: Option<Entity<SearchView>>,
        folder: Option<PathBuf>,
        cx: &mut Context<T>,
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
                tag_field_text(search.tag_values()),
            )
        });
        let (scope, matching, text, hidden, exclusions, predicates, tags) = match &search_state {
            Some((scope, matching, text, hidden, exclusions, predicates, tags)) => {
                (*scope, *matching, text.clone(), *hidden, *exclusions, predicates.clone(), tags.clone())
            }
            None => existing.as_ref().map_or(
                (
                    SavedScope::Folder,
                    SavedMatch::All,
                    String::new(),
                    (true, false),
                    SavedExclusions::Configured,
                    (None, None),
                    String::new(),
                ),
                |saved| {
                    (
                        saved.scope,
                        saved.matching,
                        saved.text.clone(),
                        (saved.include_hidden, saved.enter_hidden_folders),
                        saved.exclusions,
                        (saved.kind.clone(), saved.modified_within_days),
                        tag_field_text(&saved.tags),
                    )
                },
            ),
        };
        let name = existing.as_ref().map_or_else(
            || {
                let query = text.trim();
                if query.is_empty() && search.is_some() { "Saved Search".to_owned() } else { query.to_owned() }
            },
            |saved| saved.name.clone(),
        );
        let root = existing.as_ref().and_then(|saved| saved.root.clone()).or_else(|| {
            (scope == SavedScope::Folder)
                .then(|| folder.map(|folder| crow_core::abbreviate_home(&folder, &cx.global::<Ui>().home)))
                .flatten()
        });
        let (kind, days) = predicates;
        let focus = cx.focus_handle();
        let name = field(name, "Name", "Saved search name", cx);
        let root = field(root.unwrap_or_default(), "~/Documents", "Scope folder", cx);
        let text = field(text, "Text or empty with a predicate", "Search text", cx);
        let kind =
            field(kind.map(|kind| kind.key().to_owned()).unwrap_or_default(), "pdf, image, rs…", "Kind predicate", cx);
        let days = field(days.map(|days| days.to_string()).unwrap_or_default(), "30", "Modified within days", cx);
        let tags = field(tags, "Work, review", "Tag predicates", cx);
        Self {
            focus,
            id: existing.as_ref().map(|saved| saved.id.clone()),
            name,
            root,
            text,
            kind,
            days,
            tags,
            scope,
            matching,
            include_hidden: hidden.0,
            enter_hidden_folders: hidden.1,
            exclusions,
            error: None,
            writing: false,
            controls: [
                FormChange::Scope(SavedScope::Folder),
                FormChange::Scope(SavedScope::Home),
                FormChange::Scope(SavedScope::ThisMac),
                FormChange::Match(SavedMatch::All),
                FormChange::Match(SavedMatch::Names),
                FormChange::Match(SavedMatch::Contents),
                FormChange::IncludeHidden,
                FormChange::EnterHiddenFolders,
                FormChange::Exclusions(SavedExclusions::Configured),
                FormChange::Exclusions(SavedExclusions::Everything),
                FormChange::Remove,
                FormChange::Cancel,
                FormChange::Save,
            ]
            .map(|action| (action, cx.focus_handle())),
            subscriptions: Vec::new(),
        }
    }

    pub(crate) fn spec(&self, cx: &App) -> Result<SavedQuery, String> {
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
        let tags_text = self.tags.read(cx).text().trim().to_owned();
        let tags = parse_tags_field(&tags_text)?;
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
            tags,
        };
        query.validate(&cx.global::<Ui>().home)?;
        Ok(query)
    }
    pub(crate) fn subscribe<T: 'static>(
        &mut self,
        dispatch: fn(&mut T, FormChange, &mut Window, &mut Context<T>),
        window: &mut Window,
        cx: &mut Context<T>,
    ) {
        for field in [&self.name, &self.root, &self.text, &self.kind, &self.days, &self.tags] {
            self.subscriptions.push(cx.subscribe_in(field, window, move |host, _, event, window, cx| {
                let action = match event {
                    TextInputEvent::Confirmed => FormChange::Save,
                    TextInputEvent::Cancelled => FormChange::Cancel,
                    TextInputEvent::CompletionRequested => FormChange::Next(1),
                    _ => return,
                };
                dispatch(host, action, window, cx);
            }));
        }
    }

    pub(super) fn focus_surfaces(&self, cx: &App) -> Vec<FocusHandle> {
        let mut stops = vec![self.name.read(cx).focus_handle(cx)];
        stops.extend(self.controls[..3].iter().map(|(_, focus)| focus.clone()));
        if self.scope == SavedScope::Folder {
            stops.push(self.root.read(cx).focus_handle(cx));
        }
        stops.extend([&self.text, &self.kind, &self.days, &self.tags].map(|input| input.read(cx).focus_handle(cx)));
        stops.extend(
            self.controls[3..]
                .iter()
                .filter(|(action, _)| *action != FormChange::Remove || self.id.is_some())
                .map(|(_, focus)| focus.clone()),
        );
        stops
    }

    pub(crate) fn apply(&mut self, action: FormChange, window: &mut Window, cx: &mut App) -> FormChange {
        if action == FormChange::Activate {
            let Some((action, _)) = self.controls.iter().find(|(_, focus)| focus.is_focused(window)) else {
                return action;
            };
            return self.apply(*action, window, cx);
        }
        if let FormChange::Next(delta) = action {
            let stops = self.focus_surfaces(cx);
            let current = stops.iter().position(|focus| focus.is_focused(window)).unwrap_or(0);
            let next = (current as isize + delta).rem_euclid(stops.len() as isize) as usize;
            window.focus(&stops[next], cx);
            return action;
        }
        if !self.writing {
            match action {
                FormChange::Scope(scope) => {
                    self.scope = scope;
                    if scope == SavedScope::ThisMac {
                        self.matching = SavedMatch::Names;
                    }
                }
                FormChange::Match(matching) => self.matching = matching,
                FormChange::IncludeHidden => self.include_hidden = !self.include_hidden,
                FormChange::EnterHiddenFolders => self.enter_hidden_folders = !self.enter_hidden_folders,
                FormChange::Exclusions(exclusions) => self.exclusions = exclusions,
                _ => {}
            }
        }
        action
    }
}

impl CrowWindow {
    pub(super) fn save_search(&mut self, _: &SaveSearch, window: &mut Window, cx: &mut Context<Self>) {
        let Some(search) = self.active_search(cx) else { return };
        let id = search.read(cx).saved_query_id();
        self.open_saved_query_form(id, Some(search), window, cx);
    }

    pub(crate) fn edit_saved_search(&mut self, _: &EditSavedSearch, window: &mut Window, cx: &mut Context<Self>) {
        let id = self
            .active_search(cx)
            .and_then(|search| search.read(cx).saved_query_id())
            .or_else(|| self.sidebar.read(cx).cursor_saved_query(cx));
        if let Some(id) = id {
            self.open_saved_query_form(Some(id), None, window, cx);
        }
    }

    pub(crate) fn remove_saved_search(&mut self, _: &RemoveSavedSearch, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(crate) fn edit_saved_search_by_id(&mut self, id: SavedQueryId, window: &mut Window, cx: &mut Context<Self>) {
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
        if id.is_some() && existing.is_none() {
            return;
        }
        let mut form = SavedQueryForm::new(existing, search, folder, cx);
        form.subscribe(Self::saved_form_action, window, cx);
        let focus = form.name.read(cx).focus_handle(cx);
        self.sheet = Some(Sheet::SavedQuery(Box::new(form)));
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(crate) fn confirm_saved_query_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(crate) fn remove_saved_query_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Sheet::SavedQuery(form)) = &self.sheet else { return };
        let Some(id) = form.id.clone() else { return };
        ui::remove_saved_query(id, None, cx);
        self.sheet = None;
        self.focus_center(window, cx);
        cx.notify();
    }

    fn saved_form_action(&mut self, action: FormChange, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Sheet::SavedQuery(form)) = &mut self.sheet else { return };
        match form.apply(action, window, cx) {
            FormChange::Save => self.confirm_saved_query_form(window, cx),
            FormChange::Remove => self.remove_saved_query_form(window, cx),
            FormChange::Cancel => self.close_sheet(window, cx),
            _ => cx.notify(),
        }
    }
}

impl SavedQueryForm {
    pub(crate) fn render<T: 'static>(
        &self,
        window: &Window,
        cx: &mut Context<T>,
        dispatch: fn(&mut T, FormChange, &mut Window, &mut Context<T>),
    ) -> AnyElement {
        let form = self;
        let theme = cx.global::<Ui>().theme.clone();
        let muted = color(theme.role("fg-muted"));
        let label = |text: &'static str| div().w(px(110.)).flex_none().text_color(muted).child(text);
        let row = |label_text: &'static str, input: Entity<TextInput>| {
            div().flex().items_center().gap_3().child(label(label_text)).child(div().flex_1().child(input))
        };
        let control = |element: gpui::Stateful<gpui::Div>, action| {
            let (_, focus) = self.controls.iter().find(|(candidate, _)| *candidate == action).expect("form control");
            let handle = focus.clone();
            element
                .track_focus(focus)
                .when(focus.is_focused(window), |el| el.shadow(crate::style::cursor_ring(&theme)))
                .cursor_pointer()
                .on_click(cx.listener(move |host, _, window, cx| {
                    window.focus(&handle, cx);
                    dispatch(host, action, window, cx);
                }))
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
                .aria_toggled(if on { gpui::Toggled::True } else { gpui::Toggled::False })
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
        let choice = |id, label, selected, action| control(choice(id, label, selected), action);
        let toggle = |id, label, selected, action| control(toggle(id, label, selected), action);
        let button = |id, label, primary, action| control(button(id, label, primary), action);
        let option_row = |title, choices: Vec<gpui::Stateful<gpui::Div>>| {
            div().flex().items_center().gap_3().child(label(title)).child(div().flex().gap_1().children(choices))
        };
        div()
            .id("saved-query-form")
            .debug_selector(|| "saved-query-form".into())
            .role(Role::Dialog)
            .aria_label(if form.id.is_some() { "Edit Saved Search" } else { "Save Search" })
            .key_context("SavedQueryForm")
            .track_focus(&self.focus)
            .on_action(
                cx.listener(move |host, _: &FormNext, window, cx| dispatch(host, FormChange::Next(1), window, cx)),
            )
            .on_action(
                cx.listener(move |host, _: &FormPrevious, window, cx| dispatch(host, FormChange::Next(-1), window, cx)),
            )
            .on_action(
                cx.listener(move |host, _: &FormActivate, window, cx| dispatch(host, FormChange::Activate, window, cx)),
            )
            .flex()
            .flex_col()
            .gap_2()
            .child(row("Name", form.name.clone()))
            .child(option_row(
                "Scope",
                vec![
                    choice(
                        "saved-scope-folder",
                        "Folder",
                        form.scope == SavedScope::Folder,
                        FormChange::Scope(SavedScope::Folder),
                    ),
                    choice(
                        "saved-scope-home",
                        "Home",
                        form.scope == SavedScope::Home,
                        FormChange::Scope(SavedScope::Home),
                    ),
                    choice(
                        "saved-scope-this-mac",
                        "This Mac",
                        form.scope == SavedScope::ThisMac,
                        FormChange::Scope(SavedScope::ThisMac),
                    ),
                ],
            ))
            .when(form.scope == SavedScope::Folder, |form_row| form_row.child(row("Folder", form.root.clone())))
            .child(row("Text", form.text.clone()))
            .child(row("Kind", form.kind.clone()))
            .child(row("Modified within", form.days.clone()))
            .child(row("Tags", form.tags.clone()))
            .child(option_row(
                "Match",
                vec![
                    choice(
                        "saved-match-all",
                        "All",
                        form.matching == SavedMatch::All,
                        FormChange::Match(SavedMatch::All),
                    ),
                    choice(
                        "saved-match-names",
                        "Names",
                        form.matching == SavedMatch::Names,
                        FormChange::Match(SavedMatch::Names),
                    ),
                    choice(
                        "saved-match-contents",
                        "Contents",
                        form.matching == SavedMatch::Contents,
                        FormChange::Match(SavedMatch::Contents),
                    ),
                ],
            ))
            .child(option_row(
                "Hidden",
                vec![
                    toggle(
                        "saved-hidden-include",
                        "Include hidden names",
                        form.include_hidden,
                        FormChange::IncludeHidden,
                    ),
                    toggle(
                        "saved-hidden-enter",
                        "Enter hidden folders",
                        form.enter_hidden_folders,
                        FormChange::EnterHiddenFolders,
                    ),
                ],
            ))
            .child(option_row(
                "Exclusions",
                vec![
                    choice(
                        "saved-exclusions-configured",
                        "exclude.toml",
                        form.exclusions == SavedExclusions::Configured,
                        FormChange::Exclusions(SavedExclusions::Configured),
                    ),
                    choice(
                        "saved-exclusions-everything",
                        "Everything",
                        form.exclusions == SavedExclusions::Everything,
                        FormChange::Exclusions(SavedExclusions::Everything),
                    ),
                ],
            ))
            .when_some(form.error.clone(), |form, error| {
                form.child(div().text_size(px(12.)).text_color(color(theme.role("error"))).child(error))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pt_1()
                    .when(form.id.is_some(), |buttons| {
                        buttons.child(button("saved-remove", "Remove", false, FormChange::Remove))
                    })
                    .child(div().flex_1())
                    .child(button("saved-cancel", "Cancel", false, FormChange::Cancel))
                    .child(button("saved-save", "Save", true, FormChange::Save)),
            )
            .into_any_element()
    }
}

gpui::actions!(saved_query_form, [FormNext, FormPrevious, FormActivate]);

pub(crate) fn keymap() -> Vec<gpui::KeyBinding> {
    use gpui::KeyBinding;
    vec![
        KeyBinding::new("tab", FormNext, Some("SavedQueryForm && !TextInput")),
        KeyBinding::new("shift-tab", FormPrevious, Some("SavedQueryForm")),
        KeyBinding::new("enter", FormActivate, Some("SavedQueryForm && !TextInput")),
        KeyBinding::new("space", FormActivate, Some("SavedQueryForm && !TextInput")),
    ]
}

impl CrowWindow {
    pub(super) fn render_saved_query_form(
        &self,
        form: &SavedQueryForm,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        form.render(window, cx, Self::saved_form_action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_comma_inside_a_tag_uses_a_json_array() {
        assert_eq!(tag_field_text(&["Work".into(), "review".into()]), "Work, review");
        assert_eq!(tag_field_text(&["Work".into(), "a,b".into()]), r#"["Work","a,b"]"#);
    }

    #[test]
    fn tags_round_trip_through_the_field() {
        for tags in [
            vec!["[Urgent]".to_owned(), "Work".to_owned()],
            vec!["a,b".to_owned()],
            vec!["Work".to_owned(), "review".to_owned()],
            vec!["[".to_owned()],
        ] {
            let text = tag_field_text(&tags);
            assert_eq!(parse_tags_field(&text).unwrap(), tags, "{text}");
        }
        assert_eq!(parse_tags_field("[Urgent], Work").unwrap(), ["[Urgent]", "Work"], "hand-typed brackets are tags");
        assert_eq!(parse_tags_field("").unwrap(), Vec::<String>::new());
    }
}
