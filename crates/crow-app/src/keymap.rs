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

/// Every binding in force for one layer: the defaults with the user's keys.
pub fn effective(layer: KeymapLayer, cx: &App) -> Vec<KeyBinding> {
    let overrides = cx.try_global::<Ui>().map(|ui| ui.keys.clone()).unwrap_or_default();
    match layer {
        KeymapLayer::Finder => apply_layer(crate::actions::finder_keymap(), layer, &overrides.finder, cx).0,
        KeymapLayer::Vim => apply_layer(crate::actions::vim_keymap(), layer, &overrides.vim, cx).0,
    }
}

/// One line of Settings ▸ Keyboard.
#[derive(Clone, Debug)]
pub struct ShortcutRow {
    pub id: CommandId,
    pub label: &'static str,
    /// The menu the command lives in, for grouping and search.
    pub group: &'static str,
    /// Normalised key sequences, empty when unbound.
    pub keys: Vec<String>,
    /// Whether `keys.toml` overrides this command in the layer.
    pub changed: bool,
}

fn keys_of(bindings: &[KeyBinding], command: &crate::palette::Command) -> Vec<String> {
    bindings.iter().filter(|b| b.action().partial_eq(command.action)).map(key_text).collect()
}

/// Every command with the keys it has in `layer`, in menu order.
pub fn rows(layer: KeymapLayer, cx: &App) -> Vec<ShortcutRow> {
    let bindings = effective(layer, cx);
    let overrides = cx.try_global::<Ui>().map(|ui| ui.keys.layer(layer).clone()).unwrap_or_default();
    COMMANDS
        .iter()
        .map(|command| ShortcutRow {
            id: command.id,
            label: command.label,
            group: command.menu.map_or("Commands", |slot| slot.menu.title()),
            keys: keys_of(&bindings, command),
            changed: overrides.contains_key(command.id.key()),
        })
        .collect()
}

/// Another command (or navigation key) that a new shortcut would collide with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub label: String,
    /// The command, when it is one the editor can change.
    pub command: Option<CommandId>,
    /// The key text that collides, normalised.
    pub key: String,
}

/// Whether two predicates can both hold in one of the real focus stacks.
fn overlap(a: Option<&Rc<KeyBindingContextPredicate>>, b: Option<&Rc<KeyBindingContextPredicate>>) -> bool {
    use crate::keys::context_stack;
    const SCOPES: [KeyScope; 13] = [
        KeyScope::Window,
        KeyScope::List,
        KeyScope::Columns,
        KeyScope::ListFiltered,
        KeyScope::ColumnsFiltered,
        KeyScope::Sidebar,
        KeyScope::Filter,
        KeyScope::PathBar,
        KeyScope::Search,
        KeyScope::SearchResults,
        KeyScope::Palette,
        KeyScope::ThemePicker,
        KeyScope::Sheet,
    ];
    [KeymapLayer::Finder, KeymapLayer::Vim].into_iter().any(|layer| {
        SCOPES.into_iter().any(|scope| {
            let stack = context_stack(scope, layer);
            a.is_none_or(|p| p.eval(&stack)) && b.is_none_or(|p| p.eval(&stack))
        })
    })
}

/// The bindings that `sequence` would collide with if given to `command` in `layer`.
pub fn conflicts(layer: KeymapLayer, command: CommandId, sequence: &str, cx: &App) -> Vec<Conflict> {
    let Some(this) = COMMANDS.iter().find(|c| c.id == command) else { return Vec::new() };
    let mine = effective(layer, cx);
    let predicate = mine
        .iter()
        .find(|b| b.action().partial_eq(this.action))
        .and_then(|b| b.predicate())
        .or_else(|| KeyBindingContextPredicate::parse(default_context(this.scope, layer)).ok().map(Rc::new));
    let mut out: Vec<Conflict> = Vec::new();
    for l in [KeymapLayer::Finder, KeymapLayer::Vim] {
        for binding in effective(l, cx) {
            if binding.action().partial_eq(this.action) || binding.action().partial_eq(&NoAction {}) {
                continue;
            }
            let key = key_text(&binding);
            if !crow_core::keymap_edit::sequences_conflict(sequence, &key)
                || !overlap(predicate.as_ref(), binding.predicate().as_ref())
            {
                continue;
            }
            let other = COMMANDS.iter().find(|c| binding.action().partial_eq(c.action));
            let label = other.map_or_else(|| binding.action().name().to_owned(), |c| c.label.to_owned());
            let found = Conflict { label, command: other.map(|c| c.id), key };
            if !out.contains(&found) {
                out.push(found);
            }
        }
    }
    out
}

/// A binding that only draws `sequence`: the caps and spoken form for a row.
pub fn display(sequence: &str, cx: &App) -> Option<KeyBinding> {
    KeyBinding::load(sequence, Box::new(NoAction {}), None, false, None, cx).ok()
}

/// Commands whose key collides with another binding that can be active with it.
pub fn duplicates(layer: KeymapLayer, cx: &App) -> std::collections::HashSet<CommandId> {
    let mut out = std::collections::HashSet::new();
    let bindings = effective(layer, cx);
    let others: Vec<(KeymapLayer, Vec<KeyBinding>)> =
        [KeymapLayer::Finder, KeymapLayer::Vim].into_iter().map(|l| (l, effective(l, cx))).collect();
    for command in COMMANDS {
        let mine: Vec<&KeyBinding> = bindings.iter().filter(|b| b.action().partial_eq(command.action)).collect();
        'command: for binding in mine {
            let key = key_text(binding);
            for (_, list) in &others {
                for other in list {
                    if other.action().partial_eq(command.action) || other.action().partial_eq(&NoAction {}) {
                        continue;
                    }
                    if crow_core::keymap_edit::sequences_conflict(&key, &key_text(other))
                        && overlap(binding.predicate().as_ref(), other.predicate().as_ref())
                    {
                        out.insert(command.id);
                        break 'command;
                    }
                }
            }
        }
    }
    out
}
