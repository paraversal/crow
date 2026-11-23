//! The Preview providers editor: one ordered list at the Defaults, Folder or
//! Path Rule layer. Reads and writes go through `crow-core`'s typed layer
//! helpers, so comments and sibling fields survive; providers are only listed
//! from published descriptors, never run to populate the editor.

use std::path::PathBuf;

use crow_core::ExtensionKind;
use crow_theme::Theme;
use gpui::{App, Context, FontWeight, IntoElement, PathPromptOptions, Role, SharedString, Window, div, prelude::*, px};

use super::{ControlId, PreviewScope, SettingsWindow, callback, select_callback};
use crate::controls::{self, ControlMeta};
use crate::style::{active_selection, color, cursor_ring};
use crate::ui::{self, Ui};

#[derive(Default)]
struct ListState {
    /// The providers the list shows: the layer's own list, or the inherited one
    /// when this layer has no `preview_providers`.
    ids: Vec<String>,
    /// The layer's own list; `None` inherits.
    raw: Option<Vec<String>>,
    /// Where the displayed list comes from when `raw` is `None`.
    inherited: Option<String>,
}

impl SettingsWindow {
    pub(super) fn preview_scope_index(&self) -> usize {
        match self.preview_scope {
            PreviewScope::Defaults => 0,
            PreviewScope::Folder => 1,
            PreviewScope::Rule => 2,
        }
    }

    fn preview_rules(&self, cx: &App) -> Vec<crow_core::RuleInfo> {
        cx.global::<Ui>().views.rules()
    }

    fn current_rule(&self, cx: &App) -> Option<&crow_core::RuleLocator> {
        let locator = self.preview_rule.as_ref()?;
        (cx.global::<Ui>().views.rule_locator(locator.index).as_ref() == Some(locator)).then_some(locator)
    }

    fn rule_target_ready(&self, cx: &App) -> bool {
        self.preview_rule_new || self.current_rule(cx).is_some()
    }

    /// A rule selected earlier can move or change outside the app. Switch to a
    /// fresh new-rule target instead of aiming at the new occupant of the index.
    pub(super) fn repair_preview_target(&mut self, cx: &App) {
        if self.preview_scope != PreviewScope::Rule || self.preview_write_pending {
            return;
        }
        if self.preview_rule.is_some() {
            if self.current_rule(cx).is_none() {
                self.preview_rule = None;
                self.preview_rule_new = true;
                self.preview_new_providers.clear();
                self.preview_message = Some("The selected rule changed; choose it again.".into());
            }
        } else if !self.preview_rule_new {
            let rules = self.preview_rules(cx);
            if let Some(first) = rules.first() {
                self.preview_rule = cx.global::<Ui>().views.rule_locator(first.index);
            } else {
                self.preview_rule_new = true;
            }
        }
        let len = self.preview_list_state(cx).ids.len();
        if len == 0 {
            self.preview_list_cursor = 0;
        } else {
            self.preview_list_cursor = self.preview_list_cursor.min(len - 1);
        }
    }

    fn preview_list_state(&self, cx: &App) -> ListState {
        let views = &cx.global::<Ui>().views;
        match self.preview_scope {
            PreviewScope::Defaults => {
                let raw = views.defaults_layer().preview_providers.clone();
                let ids = raw.clone().unwrap_or_default();
                ListState { ids, raw, inherited: None }
            }
            PreviewScope::Folder => {
                let Some(folder) = self.preview_folder.as_ref() else { return ListState::default() };
                let raw = views.folder_layer(folder).and_then(|layer| layer.preview_providers.clone());
                let (resolved, source) = views.resolve(folder).preview_providers;
                let ids = raw.clone().unwrap_or_else(|| resolved.clone());
                ListState {
                    ids,
                    raw,
                    inherited: Some(format!("Inherited from {source}: {}", describe_providers(&resolved))),
                }
            }
            PreviewScope::Rule => {
                if self.preview_rule_new {
                    return ListState {
                        ids: self.preview_new_providers.clone(),
                        raw: Some(self.preview_new_providers.clone()),
                        inherited: None,
                    };
                }
                let Some(rule) = self.current_rule(cx) else { return ListState::default() };
                let raw = rule.layer.preview_providers.clone();
                let defaults = views.defaults_layer().preview_providers.clone().unwrap_or_default();
                let ids = raw.clone().unwrap_or_else(|| defaults.clone());
                ListState {
                    ids,
                    raw,
                    inherited: Some(format!(
                        "Inherited for matching folders: {} from [defaults]; other matching rules may override it",
                        describe_providers(&defaults)
                    )),
                }
            }
        }
    }

    pub(super) fn preview_list_state_len(&self, cx: &App) -> usize {
        self.preview_list_state(cx).ids.len()
    }

    #[cfg(test)]
    pub(super) fn preview_state_for_tests(
        &self,
        cx: &App,
    ) -> (PreviewScope, Option<PathBuf>, Option<usize>, bool, Vec<String>) {
        (
            self.preview_scope,
            self.preview_folder.clone(),
            self.preview_rule.as_ref().map(|locator| locator.index),
            self.preview_rule_new,
            self.preview_list_state(cx).ids,
        )
    }

    #[cfg(test)]
    pub(super) fn preview_available_for_tests(&self, cx: &App) -> Vec<String> {
        self.preview_add_options(cx).into_iter().map(|(id, _)| id).collect()
    }

    fn preview_can_clear(&self, cx: &App) -> bool {
        if self.preview_scope == PreviewScope::Rule && self.preview_rule_new {
            return false;
        }
        self.preview_list_state(cx).raw.is_some()
    }

    fn target_ready(&self, cx: &App) -> bool {
        match self.preview_scope {
            PreviewScope::Defaults => true,
            PreviewScope::Folder => self.preview_folder.is_some(),
            PreviewScope::Rule => self.rule_target_ready(cx),
        }
    }

    /// Whether a control belongs in this scope's current layout. Tab order and
    /// the rendered page both ask here.
    pub(super) fn preview_control_visible(&self, id: ControlId, cx: &App) -> bool {
        match id {
            ControlId::PreviewCurrentFolder | ControlId::PreviewChooseFolder => {
                self.preview_scope == PreviewScope::Folder
            }
            ControlId::PreviewRuleSelect => {
                self.preview_scope == PreviewScope::Rule && !self.preview_rules(cx).is_empty()
            }
            ControlId::PreviewRulePattern | ControlId::PreviewAddRule => {
                self.preview_scope == PreviewScope::Rule && self.preview_rule_new
            }
            ControlId::PreviewList
            | ControlId::PreviewMoveUp
            | ControlId::PreviewMoveDown
            | ControlId::PreviewRemove
            | ControlId::PreviewAdd => self.target_ready(cx),
            ControlId::PreviewClear => self.preview_scope != PreviewScope::Rule || !self.preview_rule_new,
            _ => true,
        }
    }

    /// Context-dependent enabled state, on top of the backing file's.
    pub(super) fn preview_control_enabled(&self, id: ControlId, cx: &App) -> bool {
        if !id.enabled(cx) || !self.preview_control_visible(id, cx) {
            return false;
        }
        match id {
            ControlId::PreviewCurrentFolder => cx.global::<Ui>().settings_folder.is_some(),
            ControlId::PreviewAddRule => {
                self.preview_scope == PreviewScope::Rule
                    && self.preview_rule_new
                    && !self.preview_pattern_text.trim().is_empty()
                    && !self.preview_new_providers.is_empty()
            }
            ControlId::PreviewMoveUp => self.preview_list_cursor > 0,
            ControlId::PreviewMoveDown => {
                let len = self.preview_list_state(cx).ids.len();
                self.preview_list_cursor + 1 < len
            }
            ControlId::PreviewRemove => !self.preview_list_state(cx).ids.is_empty(),
            ControlId::PreviewAdd => !self.preview_add_options(cx).is_empty(),
            ControlId::PreviewClear => self.preview_can_clear(cx),
            ControlId::PreviewRulePattern => self.preview_scope == PreviewScope::Rule && self.preview_rule_new,
            _ => true,
        }
    }

    fn preview_add_options(&self, cx: &App) -> Vec<(String, String)> {
        let current = self.preview_list_state(cx).ids;
        visible_preview_providers(cx)
            .into_iter()
            .filter(|(id, _)| !current.iter().any(|configured| configured == id))
            .collect()
    }

    pub(super) fn set_preview_scope(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let scope = match index {
            0 => PreviewScope::Defaults,
            1 => PreviewScope::Folder,
            _ => PreviewScope::Rule,
        };
        if scope == self.preview_scope {
            return;
        }
        self.commit_focused(window, cx);
        self.preview_scope = scope;
        self.preview_list_cursor = 0;
        self.preview_message = None;
        if scope == PreviewScope::Rule && self.preview_rule.is_none() && !self.preview_rule_new {
            let rules = self.preview_rules(cx);
            if let Some(first) = rules.first() {
                self.preview_rule = cx.global::<Ui>().views.rule_locator(first.index);
            } else {
                self.preview_rule_new = true;
            }
        }
        cx.notify();
    }

    pub(super) fn preview_use_current_folder(&mut self, cx: &mut Context<Self>) {
        let Some(folder) = cx.global::<Ui>().settings_folder.clone() else { return };
        self.set_preview_folder(folder, cx);
    }

    pub(super) fn set_preview_folder(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        self.preview_folder = Some(folder);
        self.preview_scope = PreviewScope::Folder;
        self.preview_list_cursor = 0;
        self.preview_message = None;
        cx.notify();
    }

    pub(super) fn choose_preview_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.preview_picker_pending {
            return;
        }
        self.preview_picker_pending = true;
        cx.notify();
        let origin = (self.category, self.query.clone(), self.preview_folder.clone());
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose a folder for preview providers".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = receiver.await;
            this.update_in(cx, |this, window, cx| {
                this.preview_picker_pending = false;
                if this.preview_scope != PreviewScope::Folder
                    || (this.category, &this.query, &this.preview_folder) != (origin.0, &origin.1, &origin.2)
                    || !this.visible_controls(cx).contains(&ControlId::PreviewChooseFolder)
                {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(folder) = paths.into_iter().next() {
                            this.set_preview_folder(folder, cx);
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => {
                        this.preview_message = Some(format!("Couldn't open the folder picker: {error}").into());
                    }
                    Err(error) => {
                        this.preview_message = Some(format!("Couldn't open the folder picker: {error}").into());
                    }
                }
                // The native panel took key focus; put it back on the button that
                // opened it, whether the picker was accepted or cancelled.
                let handle = this.handle(ControlId::PreviewChooseFolder);
                window.focus(&handle, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn can_write_preview(&self, cx: &App) -> bool {
        !self.preview_write_pending && cx.global::<Ui>().views_error().is_none()
    }

    fn preview_completion(
        &mut self,
        updated_rule: Option<crow_core::RuleLocator>,
        cx: &mut Context<Self>,
    ) -> ui::ViewsWriteCompletion {
        self.preview_write_pending = true;
        self.preview_message = None;
        cx.notify();
        let entity = cx.entity().downgrade();
        Box::new(move |outcome, cx| {
            entity
                .update(cx, |this, cx| {
                    this.preview_write_pending = false;
                    match outcome {
                        Ok(()) => {
                            if let Some(rule) = updated_rule
                                && this.preview_rule.as_ref().is_some_and(|selected| {
                                    selected.index == rule.index && selected.pattern == rule.pattern
                                })
                            {
                                this.preview_rule = Some(rule);
                            }
                        }
                        Err(error) => this.preview_message = Some(error.into()),
                    }
                    cx.notify();
                })
                .ok();
        })
    }

    fn commit_preview_list(&mut self, providers: Option<Vec<String>>, cx: &mut Context<Self>) {
        match self.preview_scope {
            PreviewScope::Defaults => {
                let change = providers.map_or(
                    crow_core::DefaultsChange::InheritPreviewProviders,
                    crow_core::DefaultsChange::PreviewProviders,
                );
                let completion = self.preview_completion(None, cx);
                ui::save_defaults_change(change, Some(completion), cx);
            }
            PreviewScope::Folder => {
                let Some(folder) = self.preview_folder.clone() else { return };
                let completion = self.preview_completion(None, cx);
                ui::save_folder_preview_providers(folder, providers, completion, cx);
            }
            PreviewScope::Rule if self.preview_rule_new => {
                self.preview_new_providers = providers.unwrap_or_default();
                cx.notify();
            }
            PreviewScope::Rule => {
                let Some(locator) = self.current_rule(cx).cloned() else { return };
                let mut updated = locator.clone();
                updated.layer.preview_providers = providers.clone();
                let completion = self.preview_completion(Some(updated), cx);
                ui::save_rule_preview_providers(locator, providers, completion, cx);
            }
        }
    }

    pub(super) fn preview_add_provider(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.can_write_preview(cx) || !self.preview_add_options(cx).iter().any(|(available, _)| available == &id) {
            return;
        }
        let mut providers = self.preview_list_state(cx).ids;
        providers.push(id);
        self.commit_preview_list(Some(providers), cx);
    }

    pub(super) fn preview_add_provider_at(&mut self, ids: &[String], index: usize, cx: &mut Context<Self>) {
        let Some(id) = ids.get(index).cloned() else { return };
        self.preview_add_provider(id, cx);
    }

    pub(super) fn preview_remove_provider(&mut self, cx: &mut Context<Self>) {
        if !self.can_write_preview(cx) {
            return;
        }
        let mut providers = self.preview_list_state(cx).ids;
        if providers.is_empty() {
            return;
        }
        let index = self.preview_list_cursor.min(providers.len() - 1);
        providers.remove(index);
        self.preview_list_cursor = index.min(providers.len().saturating_sub(1));
        self.commit_preview_list(Some(providers), cx);
    }

    pub(super) fn preview_move_provider(&mut self, delta: isize, cx: &mut Context<Self>) {
        if !self.can_write_preview(cx) {
            return;
        }
        let mut providers = self.preview_list_state(cx).ids;
        if providers.len() < 2 {
            return;
        }
        let current = self.preview_list_cursor.min(providers.len() - 1);
        let next = (current as isize + delta).clamp(0, providers.len() as isize - 1) as usize;
        if current == next {
            return;
        }
        providers.swap(current, next);
        self.preview_list_cursor = next;
        self.commit_preview_list(Some(providers), cx);
    }

    pub(super) fn preview_clear(&mut self, cx: &mut Context<Self>) {
        if self.can_write_preview(cx) {
            self.commit_preview_list(None, cx);
        }
    }

    pub(super) fn preview_add_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_write_preview(cx) || self.preview_scope != PreviewScope::Rule || !self.preview_rule_new {
            return;
        }
        let pattern = self.preview_pattern_text.trim().to_owned();
        if pattern.is_empty() {
            self.preview_message = Some("A path rule needs a pattern.".into());
            cx.notify();
            return;
        }
        if self.preview_new_providers.is_empty() {
            self.preview_message = Some("Add at least one provider before saving the rule.".into());
            cx.notify();
            return;
        }
        let index = self.preview_rules(cx).len();
        let providers = self.preview_new_providers.clone();
        let handle = window.window_handle().downcast::<SettingsWindow>().expect("Settings window");
        self.preview_write_pending = true;
        self.preview_message = None;
        cx.notify();
        let completion_pattern = pattern.clone();
        let completion_providers = providers.clone();
        ui::add_preview_rule(
            index,
            pattern,
            providers,
            Box::new(move |outcome, cx| {
                handle
                    .update(cx, |this, window, cx| {
                        this.preview_write_pending = false;
                        match outcome {
                            Ok(())
                                if this.preview_scope == PreviewScope::Rule
                                    && this.preview_rule_new
                                    && this.preview_pattern_text == completion_pattern =>
                            {
                                let selected = cx.global::<Ui>().views.rule_locator(index).filter(|rule| {
                                    rule.pattern == completion_pattern
                                        && rule.layer.preview_providers.as_ref() == Some(&completion_providers)
                                });
                                if let Some(selected) = selected {
                                    let restore_focus = matches!(
                                        this.focused_control(window),
                                        Some(ControlId::PreviewRulePattern | ControlId::PreviewAddRule)
                                    );
                                    this.preview_rule = Some(selected);
                                    this.preview_rule_new = false;
                                    this.preview_pattern_text.clear();
                                    this.preview_new_providers.clear();
                                    this.preview_pattern.update(cx, |input, cx| input.set_text("", window, cx));
                                    if restore_focus {
                                        window.focus(&this.handle(ControlId::PreviewRuleSelect), cx);
                                    }
                                }
                            }
                            Ok(()) => {}
                            Err(error) => this.preview_message = Some(error.into()),
                        }
                        cx.notify();
                    })
                    .ok();
            }),
            cx,
        );
    }

    pub(super) fn on_preview_pattern_event(
        &mut self,
        event: &crate::text_input::TextInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            crate::text_input::TextInputEvent::Changed => {
                self.preview_pattern_text = self.preview_pattern.read(cx).text().trim().to_owned();
                cx.notify();
            }
            crate::text_input::TextInputEvent::Confirmed => self.preview_add_rule(window, cx),
            crate::text_input::TextInputEvent::CompletionRequested => self.advance_focus(1, window, cx),
            crate::text_input::TextInputEvent::Cancelled => self.cancel(window, cx),
            crate::text_input::TextInputEvent::BackspaceAtStart => {}
        }
    }

    /// Labels and locators for the Path rule menu. The last entry starts a new
    /// preview-only rule.
    fn refresh_preview_rule_choices(&mut self, cx: &App) -> usize {
        let rules = self.preview_rules(cx);
        let views = &cx.global::<Ui>().views;
        let mut labels = Vec::new();
        let mut locators = Vec::new();
        let mut selected = None;
        for rule in &rules {
            let duplicates = rules.iter().filter(|other| other.pattern == rule.pattern).count() > 1;
            labels.push(if duplicates {
                format!("{} (rule {})", rule.pattern, rule.index + 1)
            } else {
                rule.pattern.clone()
            });
            let locator = views.rule_locator(rule.index);
            if locator.is_some() && self.preview_rule == locator {
                selected = Some(locators.len());
            }
            locators.push(locator);
        }
        labels.push("New preview rule…".to_owned());
        locators.push(None);
        if self.preview_rule_new {
            selected = Some(locators.len() - 1);
        }
        let selected = selected.unwrap_or(locators.len() - 1);
        self.preview_rule_labels = labels;
        self.preview_rule_options_cache = locators;
        selected
    }

    pub(super) fn preview_rule_menu_open(&self) -> bool {
        self.menu == Some(ControlId::PreviewRuleSelect)
    }

    pub(super) fn preview_add_menu_open(&self) -> bool {
        self.menu == Some(ControlId::PreviewAdd)
    }

    pub(super) fn open_preview_rule_menu(&mut self, cx: &mut Context<Self>) {
        let selected = self.refresh_preview_rule_choices(cx);
        self.preview_rule_cursor = selected;
        self.preview_rule_scroll.scroll_to_item(selected);
        self.menu = Some(ControlId::PreviewRuleSelect);
        cx.notify();
    }

    pub(super) fn open_preview_add_menu(&mut self, cx: &mut Context<Self>) {
        (self.preview_add_ids, self.preview_add_labels) = self.preview_add_options(cx).into_iter().unzip();
        self.preview_add_cursor = 0;
        self.preview_add_scroll.scroll_to_item(0);
        self.menu = Some(ControlId::PreviewAdd);
        cx.notify();
    }

    pub(super) fn move_preview_menu(&mut self, delta: isize, cx: &mut Context<Self>) {
        match self.menu {
            Some(ControlId::PreviewRuleSelect) => {
                let last = self.preview_rule_labels.len().saturating_sub(1);
                self.preview_rule_cursor = self.preview_rule_cursor.saturating_add_signed(delta).min(last);
                self.preview_rule_scroll.scroll_to_item(self.preview_rule_cursor);
            }
            Some(ControlId::PreviewAdd) => {
                let last = self.preview_add_labels.len().saturating_sub(1);
                self.preview_add_cursor = self.preview_add_cursor.saturating_add_signed(delta).min(last);
                self.preview_add_scroll.scroll_to_item(self.preview_add_cursor);
            }
            _ => return,
        }
        cx.notify();
    }

    pub(super) fn keep_preview_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.menu {
            Some(ControlId::PreviewRuleSelect) => {
                self.choose_preview_rule(self.preview_rule_cursor, window, cx);
            }
            Some(ControlId::PreviewAdd) => {
                let id = self.preview_add_ids.get(self.preview_add_cursor).cloned();
                if let Some(id) = id {
                    self.preview_add_provider(id, cx);
                }
            }
            _ => {}
        }
        self.menu = None;
        cx.notify();
    }

    pub(super) fn choose_preview_rule(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let options = self.preview_rule_options_cache.clone();
        self.choose_preview_rule_at(&options, index, window, cx);
    }

    /// Select from the options the row was drawn with, so an inventory change
    /// between paint and click cannot move the click onto another rule.
    pub(super) fn choose_preview_rule_at(
        &mut self,
        options: &[Option<crow_core::RuleLocator>],
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.preview_write_pending {
            return;
        }
        let Some(option) = options.get(index).cloned() else { return };
        match option {
            Some(locator) => {
                let current = cx.global::<Ui>().views.rule_locator(locator.index).as_ref() == Some(&locator);
                if !current {
                    self.preview_rule = None;
                    self.preview_rule_new = true;
                    self.preview_message = Some("That rule changed; choose it again.".into());
                } else {
                    self.preview_rule = Some(locator);
                    self.preview_rule_new = false;
                    self.preview_message = None;
                }
            }
            None => {
                if !self.preview_rule_new {
                    self.preview_new_providers.clear();
                    self.preview_pattern_text.clear();
                    // The box shows what the draft holds: nothing.
                    self.preview_pattern.update(cx, |input, cx| input.set_text("", window, cx));
                }
                self.preview_rule = None;
                self.preview_rule_new = true;
                self.preview_message = None;
            }
        }
        self.preview_list_cursor = 0;
        cx.notify();
    }

    pub(super) fn preview_type_ahead(&mut self, id: ControlId, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        let labels = match id {
            ControlId::PreviewRuleSelect => self.preview_rule_labels.clone(),
            ControlId::PreviewAdd => self.preview_add_labels.clone(),
            _ => return,
        };
        let state = match id {
            ControlId::PreviewRuleSelect => &mut self.preview_rule_type_ahead,
            _ => &mut self.preview_add_type_ahead,
        };
        let Some(index) = super::type_ahead(state, &event.keystroke, &labels) else { return };
        if self.menu != Some(id) {
            if id == ControlId::PreviewRuleSelect {
                self.open_preview_rule_menu(cx);
            } else {
                self.open_preview_add_menu(cx);
            }
        }
        match id {
            ControlId::PreviewRuleSelect => {
                self.preview_rule_cursor = index;
                self.preview_rule_scroll.scroll_to_item(index);
            }
            ControlId::PreviewAdd => {
                self.preview_add_cursor = index;
                self.preview_add_scroll.scroll_to_item(index);
            }
            _ => {}
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn preview_inheritance_text(&self, cx: &App) -> SharedString {
        if let Some(message) = &self.preview_message {
            return message.clone();
        }
        let state = self.preview_list_state(cx);
        let text = match self.preview_scope {
            PreviewScope::Defaults => match &state.raw {
                Some(list) if list.is_empty() => "Native only (an explicit empty list).".to_owned(),
                Some(_) => "Crow tries these providers in order, then falls back to Native.".to_owned(),
                None => "Inherited: the built-in Native fallback.".to_owned(),
            },
            PreviewScope::Folder => match (&self.preview_folder, &state.raw) {
                (None, _) => "Choose a folder to see and edit its list.".to_owned(),
                (Some(_), Some(_)) => {
                    "This folder's own list. Clearing it inherits from rules and defaults again.".to_owned()
                }
                (Some(_), None) => state
                    .inherited
                    .clone()
                    .unwrap_or_else(|| "This folder inherits from the rules and defaults.".to_owned()),
            },
            PreviewScope::Rule if self.preview_rule_new => format!(
                "New rule after {} existing rules; later rules win for folders it matches.",
                self.preview_rules(cx).len()
            ),
            PreviewScope::Rule => match &state.raw {
                Some(_) => "This rule's own list. Clearing it keeps the rule's other fields.".to_owned(),
                None => {
                    state.inherited.clone().unwrap_or_else(|| "This rule sets no preview list; it inherits.".to_owned())
                }
            },
        };
        text.into()
    }

    pub(super) fn render_preview_unit(
        &mut self,
        window: &Window,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.repair_preview_target(cx);
        let list = self.preview_list_state(cx);
        let available = visible_preview_providers(cx);
        let mut rows: Vec<gpui::AnyElement> = Vec::new();

        let scope_select = select_callback(cx, |this, index, window, cx| this.set_preview_scope(index, window, cx));
        rows.push(
            controls::segmented(
                self.meta(ControlId::PreviewScope, cx),
                &["Defaults", "Folder", "Path Rule"],
                self.preview_scope_index(),
                self.style(ControlId::PreviewScope, window, theme),
                scope_select,
                None,
            )
            .into_any_element(),
        );

        match self.preview_scope {
            PreviewScope::Defaults => {
                rows.push(muted_line("Every folder without a folder or rule override.", theme));
            }
            PreviewScope::Folder => {
                let folder_text = self.preview_folder.as_ref().map_or_else(
                    || "No folder selected.".to_owned(),
                    |folder| crow_core::abbreviate_home(folder, &cx.global::<Ui>().home),
                );
                let browser = cx.global::<Ui>().settings_folder.as_ref().map(|folder| {
                    format!("Browser folder: {}", crow_core::abbreviate_home(folder, &cx.global::<Ui>().home))
                });
                rows.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .text_size(px(12.))
                        .child(
                            div()
                                .id("settings-preview-folder-target")
                                .role(Role::Label)
                                .aria_label(folder_text.clone())
                                .text_color(color(theme.role("fg")))
                                .child(folder_text),
                        )
                        .children(browser.map(|text| {
                            div()
                                .id("settings-preview-browser-folder")
                                .role(Role::Label)
                                .aria_label(text.clone())
                                .text_size(px(11.))
                                .text_color(color(theme.role("fg-muted")))
                                .child(text)
                        }))
                        .into_any_element(),
                );
                let use_current = controls::button(
                    ControlId::PreviewCurrentFolder.key(),
                    "Use Current Folder",
                    false,
                    self.preview_control_enabled(ControlId::PreviewCurrentFolder, cx),
                    self.style(ControlId::PreviewCurrentFolder, window, theme),
                    callback(cx, |this, _, cx| this.preview_use_current_folder(cx)),
                );
                let choose = controls::button(
                    ControlId::PreviewChooseFolder.key(),
                    if self.preview_picker_pending { "Choosing…" } else { "Choose Folder…" },
                    false,
                    self.preview_control_enabled(ControlId::PreviewChooseFolder, cx),
                    self.style(ControlId::PreviewChooseFolder, window, theme),
                    callback(cx, |this, window, cx| this.choose_preview_folder(window, cx)),
                );
                rows.push(div().flex().items_center().gap(px(8.)).child(use_current).child(choose).into_any_element());
            }
            PreviewScope::Rule => {
                if !self.preview_rules(cx).is_empty() {
                    let selected = self.refresh_preview_rule_choices(cx);
                    let menu = self.preview_rule_menu_open().then(|| controls::Menu {
                        cursor: self.preview_rule_cursor,
                        scroll: &self.preview_rule_scroll,
                        dismiss: callback(cx, |this, window, cx| {
                            this.close_menu(window, cx);
                            this.dismissed_by_mouse = Some(ControlId::PreviewRuleSelect);
                        }),
                    });
                    let toggle =
                        callback(cx, |this, window, cx| this.toggle_menu(ControlId::PreviewRuleSelect, window, cx));
                    let rule_options = self.preview_rule_options_cache.clone();
                    let pick = select_callback(cx, move |this, index, window, cx| {
                        this.choose_preview_rule_at(&rule_options, index, window, cx);
                        this.close_menu(window, cx);
                    });
                    let labels = self.preview_rule_labels.clone();
                    let row = controls::dropdown(
                        self.meta(ControlId::PreviewRuleSelect, cx),
                        &labels,
                        selected,
                        menu,
                        self.style(ControlId::PreviewRuleSelect, window, theme),
                        toggle,
                        pick,
                    )
                    .on_key_down(cx.listener(|this, event, _, cx| {
                        this.preview_type_ahead(ControlId::PreviewRuleSelect, event, cx)
                    }));
                    rows.push(row.into_any_element());
                }
                if self.preview_rule_new {
                    rows.push(
                        controls::text_field(
                            self.meta(ControlId::PreviewRulePattern, cx),
                            &self.preview_pattern,
                            self.style(ControlId::PreviewRulePattern, window, theme),
                            300.0,
                        )
                        .into_any_element(),
                    );
                    let add = controls::button(
                        ControlId::PreviewAddRule.key(),
                        "Add Preview Rule",
                        true,
                        self.preview_control_enabled(ControlId::PreviewAddRule, cx),
                        self.style(ControlId::PreviewAddRule, window, theme),
                        callback(cx, |this, window, cx| this.preview_add_rule(window, cx)),
                    );
                    let position = format!(
                        "Goes after {} existing rules; it wins over them for matching folders.",
                        self.preview_rules(cx).len()
                    );
                    rows.push(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(add)
                            .child(muted_line(&position, theme))
                            .into_any_element(),
                    );
                } else if let Some(rule) = self.current_rule(cx) {
                    rows.push(muted_line(
                        &format!(
                            "Editing rule {} of {}: {}",
                            rule.index + 1,
                            self.preview_rules(cx).len(),
                            rule.pattern
                        ),
                        theme,
                    ));
                }
            }
        }

        if self.target_ready(cx) {
            let mut list_rows: Vec<gpui::AnyElement> = Vec::new();
            for (index, id) in list.ids.iter().enumerate() {
                let selected = index == self.preview_list_cursor;
                let label = provider_label(id, &available);
                list_rows.push(
                    div()
                        .id(("settings-preview-provider", index))
                        .debug_selector(move || format!("settings-preview-provider-{index}"))
                        .role(Role::ListBoxOption)
                        .aria_label(label.clone())
                        .aria_selected(selected)
                        .aria_position_in_set(index + 1)
                        .aria_size_of_set(list.ids.len())
                        .when(selected, |row| row.bg(active_selection(theme)))
                        .when(selected, |row| row.text_color(color(theme.role("on-accent"))))
                        .when(!selected, |row| row.text_color(color(theme.role("fg"))))
                        .when(selected, |row| row.shadow(cursor_ring(theme)))
                        .px(px(6.))
                        .py(px(3.))
                        .rounded(px(4.))
                        .text_size(px(12.))
                        .truncate()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.preview_list_cursor = index;
                            cx.notify();
                        }))
                        .child(label)
                        .into_any_element(),
                );
            }
            let list_focus = self.handle(ControlId::PreviewList);
            let list_container = div()
                .id("settings-preview-list")
                .debug_selector(|| "settings-preview-list".to_owned())
                .role(Role::ListBox)
                .aria_label("Preview providers")
                .track_focus(&list_focus)
                .max_h(px(180.))
                .overflow_y_scroll()
                .track_scroll(&self.preview_list_scroll)
                .p(px(2.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .rounded(px(6.))
                .bg(color(theme.role("surface")))
                .children(if list_rows.is_empty() {
                    vec![
                        div()
                            .id("settings-preview-empty")
                            .role(Role::Status)
                            .aria_label("No preview providers; Native is used")
                            .px(px(6.))
                            .py(px(3.))
                            .text_size(px(12.))
                            .text_color(color(theme.role("fg-muted")))
                            .child(if list.raw.as_deref() == Some(&[]) {
                                "Native only."
                            } else {
                                "No providers; Native is used."
                            })
                            .into_any_element(),
                    ]
                } else {
                    list_rows
                });
            rows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color(theme.role("fg-muted")))
                            .child("Providers"),
                    )
                    .child(list_container)
                    .into_any_element(),
            );
            let up = controls::button(
                ControlId::PreviewMoveUp.key(),
                "Move Up",
                false,
                self.preview_control_enabled(ControlId::PreviewMoveUp, cx),
                self.style(ControlId::PreviewMoveUp, window, theme),
                callback(cx, |this, _, cx| this.preview_move_provider(-1, cx)),
            );
            let down = controls::button(
                ControlId::PreviewMoveDown.key(),
                "Move Down",
                false,
                self.preview_control_enabled(ControlId::PreviewMoveDown, cx),
                self.style(ControlId::PreviewMoveDown, window, theme),
                callback(cx, |this, _, cx| this.preview_move_provider(1, cx)),
            );
            let remove = controls::button(
                ControlId::PreviewRemove.key(),
                "Remove",
                false,
                self.preview_control_enabled(ControlId::PreviewRemove, cx),
                self.style(ControlId::PreviewRemove, window, theme),
                callback(cx, |this, _, cx| this.preview_remove_provider(cx)),
            );
            let add_options = self.preview_add_options(cx);
            self.preview_add_labels = add_options.iter().map(|(_, title)| title.clone()).collect();
            self.preview_add_ids = add_options.iter().map(|(id, _)| id.clone()).collect();
            let add_menu = self.preview_add_menu_open().then(|| controls::Menu {
                cursor: self.preview_add_cursor,
                scroll: &self.preview_add_scroll,
                dismiss: callback(cx, |this, window, cx| {
                    this.close_menu(window, cx);
                    this.dismissed_by_mouse = Some(ControlId::PreviewAdd);
                }),
            });
            let add_toggle = callback(cx, |this, window, cx| this.toggle_menu(ControlId::PreviewAdd, window, cx));
            let add_ids = self.preview_add_ids.clone();
            let add_pick = select_callback(cx, move |this, index, window, cx| {
                this.preview_add_provider_at(&add_ids, index, cx);
                this.close_menu(window, cx);
            });
            let add = controls::menu_button(
                ControlMeta {
                    id: ControlId::PreviewAdd.key(),
                    label: ControlId::PreviewAdd.label(),
                    description: None,
                    enabled: self.preview_control_enabled(ControlId::PreviewAdd, cx),
                },
                "Add Provider",
                &self.preview_add_labels.clone(),
                add_menu,
                self.style(ControlId::PreviewAdd, window, theme),
                add_toggle,
                add_pick,
            )
            .on_key_down(cx.listener(|this, event, _, cx| this.preview_type_ahead(ControlId::PreviewAdd, event, cx)));
            let clear_label =
                if self.preview_scope == PreviewScope::Defaults { "Use built-in Native" } else { "Use inherited list" };
            let clear = self.preview_control_visible(ControlId::PreviewClear, cx).then(|| {
                controls::button(
                    ControlId::PreviewClear.key(),
                    clear_label,
                    false,
                    self.preview_control_enabled(ControlId::PreviewClear, cx),
                    self.style(ControlId::PreviewClear, window, theme),
                    callback(cx, |this, _, cx| this.preview_clear(cx)),
                )
            });
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap(px(6.))
                    .child(up)
                    .child(down)
                    .child(remove)
                    .child(add)
                    .children(clear)
                    .into_any_element(),
            );
        }

        let inheritance = self.preview_inheritance_text(cx);
        let is_message = self.preview_message.is_some();
        rows.push(
            div()
                .id("settings-preview-note")
                .debug_selector(|| "settings-preview-note".to_owned())
                .role(Role::Status)
                .aria_label(inheritance.clone())
                .text_size(px(11.))
                .text_color(color(if is_message { theme.role("error") } else { theme.role("fg-muted") }))
                .child(inheritance)
                .into_any_element(),
        );

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(color(theme.role("fg")))
                    .child("Preview providers"),
            )
            .child(
                div().text_size(px(11.)).text_color(color(theme.role("fg-muted"))).child(
                    "Ordered lists at the Defaults, Folder and Path Rule layers; the first matching provider runs.",
                ),
            )
            .children(rows)
            .into_any_element()
    }
}

fn muted_line(text: &str, theme: &Theme) -> gpui::AnyElement {
    div().text_size(px(11.)).text_color(color(theme.role("fg-muted"))).child(text.to_owned()).into_any_element()
}

fn describe_providers(ids: &[String]) -> String {
    if ids.is_empty() { "Native".to_owned() } else { ids.join(", ") }
}

/// Registered preview descriptors, by title then id, with a title falling back
/// to the id. Configured ids that are not here stay visible as unavailable.
pub(super) fn visible_preview_providers(cx: &App) -> Vec<(String, String)> {
    let Some(plugins) = cx.try_global::<crate::plugins::Plugins>() else { return Vec::new() };
    let snapshot = plugins.surfaces.snapshot();
    let mut out: Vec<(String, String)> = snapshot
        .of_kind(ExtensionKind::Preview)
        .filter(|(id, _)| !plugins.surfaces.is_suppressed(&id.plugin))
        .map(|(id, published)| {
            let title = published.descriptor.title.trim();
            (id.to_string(), if title.is_empty() { id.to_string() } else { title.to_owned() })
        })
        .collect();
    out.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()).then_with(|| a.0.cmp(&b.0)));
    out
}

fn provider_label(id: &str, available: &[(String, String)]) -> String {
    let Some((_, title)) = available.iter().find(|(candidate, _)| candidate == id) else {
        return format!("{id} (unavailable)");
    };
    let duplicates = available.iter().filter(|(_, other)| other.eq_ignore_ascii_case(title)).count() > 1;
    if duplicates { format!("{title} ({id})") } else { title.clone() }
}
