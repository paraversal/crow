//! The effective keymap: Crow's defaults (`actions.rs`) with the user's
//! `keys.toml` overrides laid over them. A command with an override loses all
//! of its default bindings in that layer and gets the user's keys in the
//! context its first default binding had.

use std::collections::BTreeMap;
use std::rc::Rc;

use crow_core::KeymapLayer;
use crow_core::keymap_edit::{KeyOverrides, normalize_sequence};
use gpui::{App, KeyBinding, KeyBindingContextPredicate, KeyContext, NoAction};

use crate::keys::KeyScope;
use crate::palette::{COMMANDS, CommandId};
use crate::ui::Ui;

/// Where a command's keys apply when it had no default binding to copy from.
fn default_context(scope: KeyScope, layer: KeymapLayer) -> &'static str {
    let vim = layer == KeymapLayer::Vim;
    match (scope, vim) {
        (KeyScope::List | KeyScope::ListFiltered, false) => "FileList && !Filter",
        (KeyScope::List | KeyScope::ListFiltered, true) => "FileList && vim && !Filter && !TextInput",
        (KeyScope::Columns | KeyScope::ColumnsFiltered, false) => "ColumnView && !Filter",
        (KeyScope::Columns | KeyScope::ColumnsFiltered, true) => "ColumnView && vim && !Filter && !TextInput",
        (KeyScope::Sidebar, false) => "Sidebar",
        (KeyScope::Sidebar, true) => "Sidebar && vim",
        _ => "Crow",
    }
}

/// The bindings of `base` with `overrides` applied. Returns the bindings and a
/// message for each key GPUI would not parse.
pub fn apply_layer(
    base: Vec<KeyBinding>,
    layer: KeymapLayer,
    overrides: &BTreeMap<String, Vec<String>>,
    cx: &App,
) -> (Vec<KeyBinding>, Vec<String>) {
    let mut bindings = base;
    let mut problems = Vec::new();
    let sheet = [KeyContext::parse("Crow").expect("literal"), KeyContext::parse("Sheet").expect("literal")];
    for (id, keys) in overrides {
        let Some(command) = CommandId::from_key(id).and_then(|cid| COMMANDS.iter().find(|c| c.id == cid)) else {
            continue;
        };
        let mut predicate: Option<Rc<KeyBindingContextPredicate>> = None;
        let mut found = false;
        let mut nulled_in_sheet = false;
        let defaults: Vec<String> =
            bindings.iter().filter(|b| b.action().partial_eq(command.action)).map(key_text).collect();
        bindings.retain(|b| {
            if !b.action().partial_eq(command.action) {
                return true;
            }
            if !found {
                predicate = b.predicate();
                found = true;
            }
            false
        });
        if !found {
            predicate = KeyBindingContextPredicate::parse(default_context(command.scope, layer)).ok().map(Rc::new);
        }
        // A modal sheet nulls the window keys behind it; follow a moved key there.
        for binding in &bindings {
            let is_null = binding.action().partial_eq(&NoAction {})
                && binding.predicate().is_some_and(|p| p.eval(&sheet))
                && defaults.contains(&key_text(binding));
            nulled_in_sheet |= is_null;
        }
        for key in keys {
            match KeyBinding::load(key, command.action.boxed_clone(), predicate.clone(), false, None, cx) {
                Ok(binding) => bindings.push(binding),
                Err(error) => problems.push(format!("keys.toml: {id}: {error}")),
            }
            if nulled_in_sheet
                && let Ok(null) = KeyBinding::load(key, Box::new(NoAction {}), sheet_predicate(), false, None, cx)
            {
                bindings.push(null);
            }
        }
    }
    (bindings, problems)
}

fn sheet_predicate() -> Option<Rc<KeyBindingContextPredicate>> {
    KeyBindingContextPredicate::parse("Sheet").ok().map(Rc::new)
}

fn key_text(binding: &KeyBinding) -> String {
    let text = binding
        .keystrokes()
        .iter()
        .map(|k| k.inner().unparse(gpui::OperatingSystem::Mac))
        .collect::<Vec<_>>()
        .join(" ");
    normalize_sequence(&text).unwrap_or(text)
}

/// Clear and rebind every keymap from the defaults plus `keys.toml`.
pub fn install(cx: &mut App) {
    let overrides = cx.try_global::<Ui>().map(|ui| ui.keys.clone()).unwrap_or_default();
    install_with(&overrides, cx);
}

pub fn install_with(overrides: &KeyOverrides, cx: &mut App) {
    cx.clear_key_bindings();
    let (finder, mut problems) =
        apply_layer(crate::actions::finder_keymap(), KeymapLayer::Finder, &overrides.finder, cx);
    let (vim, more) = apply_layer(crate::actions::vim_keymap(), KeymapLayer::Vim, &overrides.vim, cx);
    problems.extend(more);
    for problem in problems {
        tracing::warn!("{problem}");
    }
    cx.bind_keys(finder);
    cx.bind_keys(vim);
    cx.bind_keys(crate::text_input::keymap());
    cx.bind_keys(crate::settings_window::keymap());
}
