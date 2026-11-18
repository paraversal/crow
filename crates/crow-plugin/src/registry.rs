//! Registration and reply validation. One malformed descriptor or field rejects
//! the whole batch; nothing partial reaches the host.

use std::collections::{BTreeMap, HashSet};

use crow_core::{
    CommandIntent, ContributionValues, Descriptor, EntryHandle, EntryValue, ExtensionId, ExtensionKind, IconMapping,
    IconPack, InfoRow, LocalId, PluginId, PluginPreview, SidebarRow, StatusItem,
};
use rhai::{AST, Array, Dynamic, Map};

use crate::dto::{CallError, Reply, RequestInput};
use crate::limits::Limits;
use crate::manifest::Manifest;

pub fn validate_registration(
    plugin: &PluginId,
    manifest: &Manifest,
    ast: &AST,
    value: Dynamic,
    limits: &Limits,
) -> Result<Vec<Descriptor>, CallError> {
    let array = as_array(&value).ok_or_else(|| CallError::invalid("register() must return an array"))?;
    if array.len() > limits.max_registrations {
        return Err(CallError::invalid(format!(
            "register() returned {} descriptors; the limit is {}",
            array.len(),
            limits.max_registrations
        )));
    }
    let mut descriptors = Vec::with_capacity(array.len());
    let mut locals = HashSet::new();
    for (index, item) in array.into_iter().enumerate() {
        let map = as_map(&item).ok_or_else(|| CallError::invalid(format!("descriptor {index} is not a map")))?;
        reject_unknown(&map, &["kind", "id", "title", "applies_to", "function"], "descriptor")?;
        let kind_key = required_string(&map, "kind", "descriptor kind")?;
        let kind = ExtensionKind::parse(&kind_key)
            .ok_or_else(|| CallError::invalid(format!("descriptor {index} has unknown kind \"{kind_key}\"")))?;
        if !manifest.provides.contains(&kind) {
            return Err(CallError::invalid(format!(
                "descriptor {index} kind \"{}\" is not declared in plugin.toml provides",
                kind.key()
            )));
        }
        let local_key = required_string(&map, "id", "descriptor id")?;
        let local = LocalId::parse(&local_key)
            .map_err(|error| CallError::invalid(format!("descriptor {index} id: {error}")))?;
        if !locals.insert(local.clone()) {
            return Err(CallError::invalid(format!("descriptor id \"{local}\" is registered twice")));
        }
        let title = required_string(&map, "title", "descriptor title")?;
        if title.trim().is_empty() {
            return Err(CallError::invalid(format!("descriptor \"{local}\" has an empty title")));
        }
        let title = checked_line(&title, "descriptor title", limits)?;
        let applies_to = match map.get("applies_to") {
            None => None,
            Some(value) => {
                let text = value_string(value, "descriptor applies_to")?;
                Some(checked_line(&text, "descriptor applies_to", limits)?)
            }
        };
        let function = match map.get("function") {
            None => None,
            Some(value) => Some(checked_function(&value_string(value, "descriptor function")?, ast)?),
        };
        match (kind, &function) {
            (ExtensionKind::IconPack, Some(_)) => {
                return Err(CallError::invalid(format!(
                    "icon-pack \"{local}\" is data-only and cannot name a function"
                )));
            }
            (ExtensionKind::IconPack, None) => {}
            (_, None) => {
                return Err(CallError::invalid(format!("descriptor \"{local}\" needs a named function")));
            }
            (_, Some(_)) => {}
        }
        descriptors.push(Descriptor { kind, id: ExtensionId::new(plugin.clone(), local), title, applies_to, function });
    }
    Ok(descriptors)
}

/// Host-side defence in depth after the worker returns descriptors.
pub fn validate_published(
    plugin: &PluginId,
    provides: &[ExtensionKind],
    descriptors: &[Descriptor],
    limits: &Limits,
) -> Result<(), String> {
    if descriptors.len() > limits.max_registrations {
        return Err(format!("more than {} descriptors", limits.max_registrations));
    }
    let mut locals = HashSet::new();
    for descriptor in descriptors {
        if &descriptor.id.plugin != plugin {
            return Err(format!("descriptor {} escaped its plugin namespace", descriptor.id));
        }
        if !provides.contains(&descriptor.kind) {
            return Err(format!("descriptor {} is not declared in provides", descriptor.id));
        }
        if !locals.insert(descriptor.id.local.clone()) {
            return Err(format!("descriptor {} is registered twice", descriptor.id));
        }
        if descriptor.title.is_empty() || descriptor.title.len() > limits.max_display_bytes {
            return Err(format!("descriptor {} has an invalid title", descriptor.id));
        }
        if descriptor.kind != ExtensionKind::IconPack && descriptor.function.as_deref().is_none_or(str::is_empty) {
            return Err(format!("descriptor {} has no function", descriptor.id));
        }
    }
    Ok(())
}

pub fn reply_for(
    descriptor: &Descriptor,
    value: Dynamic,
    input: &RequestInput,
    limits: &Limits,
) -> Result<Reply, CallError> {
    match descriptor.kind {
        ExtensionKind::Command => Ok(Reply::Intent(command_reply(value, limits)?)),
        ExtensionKind::Hook => Ok(hook_reply(value)?),
        ExtensionKind::Column | ExtensionKind::Decoration => {
            let values = entry_values(value, input, limits)?;
            Ok(Reply::Values(if descriptor.kind == ExtensionKind::Column {
                ContributionValues::Column(values)
            } else {
                ContributionValues::Decoration(values)
            }))
        }
        ExtensionKind::Preview => Ok(Reply::Values(ContributionValues::Preview(preview_reply(value, limits)?))),
        ExtensionKind::Sidebar => Ok(Reply::Values(ContributionValues::Sidebar(sidebar_reply(value, limits)?))),
        ExtensionKind::Status => Ok(Reply::Values(ContributionValues::Status(status_reply(value, limits)?))),
        ExtensionKind::Info => Ok(Reply::Values(ContributionValues::Info(info_reply(value, limits)?))),
        ExtensionKind::IconPack => Ok(Reply::Values(ContributionValues::IconPack(icon_pack_reply(value, limits)?))),
    }
}

fn command_reply(value: Dynamic, limits: &Limits) -> Result<CommandIntent, CallError> {
    let map = as_map(&value).ok_or_else(|| CallError::invalid("a command must return a map"))?;
    reject_unknown(&map, &["action", "text"], "command reply")?;
    let action = required_string(&map, "action", "command action")?;
    if action != "copy-text" {
        return Err(CallError::invalid(format!("command action \"{action}\" is not in the api 1 allowlist")));
    }
    let text = required_string(&map, "text", "command text")?;
    let text = checked_text(&text, "command text", limits)?;
    Ok(CommandIntent::CopyText(text))
}

fn hook_reply(value: Dynamic) -> Result<Reply, CallError> {
    if value.is_unit() {
        return Ok(Reply::Unit);
    }
    let map = as_map(&value).ok_or_else(|| CallError::invalid("a hook must return unit or a map"))?;
    reject_unknown(&map, &["refresh"], "hook reply")?;
    let refresh = match map.get("refresh") {
        None => false,
        Some(value) => {
            value.clone().try_cast::<bool>().ok_or_else(|| CallError::invalid("hook refresh must be a bool"))?
        }
    };
    Ok(if refresh { Reply::Refresh } else { Reply::Unit })
}

fn entry_values(value: Dynamic, input: &RequestInput, limits: &Limits) -> Result<Vec<EntryValue>, CallError> {
    let array = as_array(&value).ok_or_else(|| CallError::invalid("column/decor reply must be an array"))?;
    if array.len() > limits.max_entries_per_call {
        return Err(CallError::invalid(format!(
            "column/decor reply has {} entries; the limit is {}",
            array.len(),
            limits.max_entries_per_call
        )));
    }
    let known: HashSet<EntryHandle> = input.snapshot.entries.iter().map(|entry| entry.handle).collect();
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(array.len());
    let mut payload = 0;
    for item in array {
        let map = as_map(&item).ok_or_else(|| CallError::invalid("column/decor value is not a map"))?;
        reject_unknown(&map, &["handle", "text", "spoken"], "column/decor value")?;
        let handle = EntryHandle(required_handle(&map, "handle")?);
        if !known.contains(&handle) {
            return Err(CallError::invalid(format!("reply names unknown entry handle {}", handle.0)));
        }
        if !seen.insert(handle) {
            return Err(CallError::invalid(format!("reply names entry handle {} twice", handle.0)));
        }
        let text = checked_line(&required_string(&map, "text", "column/decor text")?, "column/decor text", limits)?;
        add_payload(&mut payload, text.len(), limits)?;
        let spoken = match map.get("spoken") {
            None => None,
            Some(value) => {
                let spoken = checked_line(&value_string(value, "column/decor spoken")?, "column/decor spoken", limits)?;
                add_payload(&mut payload, spoken.len(), limits)?;
                Some(spoken)
            }
        };
        out.push(EntryValue { handle, text, spoken });
    }
    Ok(out)
}

fn preview_reply(value: Dynamic, limits: &Limits) -> Result<PluginPreview, CallError> {
    if value.is_unit() {
        return Ok(PluginPreview::default());
    }
    let map = as_map(&value).ok_or_else(|| CallError::invalid("a preview must return unit or a map"))?;
    reject_unknown(&map, &["text", "table"], "preview reply")?;
    let text = match map.get("text") {
        None => None,
        Some(value) => Some(checked_text(&value_string(value, "preview text")?, "preview text", limits)?),
    };
    let table = match map.get("table") {
        None => None,
        Some(value) => {
            let rows = as_array(value).ok_or_else(|| CallError::invalid("preview table must be an array of rows"))?;
            if rows.len() > 128 {
                return Err(CallError::invalid("preview table has more than 128 rows"));
            }
            let mut payload = 0;
            let mut cells = 0;
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                let cells_value = as_array(&row).ok_or_else(|| CallError::invalid("preview row must be an array"))?;
                if cells_value.len() > 64 {
                    return Err(CallError::invalid("preview row has more than 64 cells"));
                }
                let mut row_out = Vec::with_capacity(cells_value.len());
                for cell in cells_value {
                    let cell = checked_line(&value_string(&cell, "preview cell")?, "preview cell", limits)?;
                    add_payload(&mut payload, cell.len(), limits)?;
                    cells += 1;
                    row_out.push(cell);
                }
                out.push(row_out);
            }
            if cells > 1024 {
                return Err(CallError::invalid("preview table has more than 1024 cells"));
            }
            Some(out)
        }
    };
    if text.is_some() && table.is_some() {
        return Err(CallError::invalid("a preview returns text or a table, not both"));
    }
    Ok(PluginPreview { text, table })
}

fn sidebar_reply(value: Dynamic, limits: &Limits) -> Result<Vec<SidebarRow>, CallError> {
    let array = as_array(&value).ok_or_else(|| CallError::invalid("sidebar reply must be an array"))?;
    if array.len() > limits.max_entries_per_call {
        return Err(CallError::invalid("sidebar reply has too many rows"));
    }
    let mut ids = HashSet::new();
    let mut rows = Vec::with_capacity(array.len());
    let mut payload = 0;
    for item in array {
        let map = as_map(&item).ok_or_else(|| CallError::invalid("sidebar row is not a map"))?;
        reject_unknown(&map, &["id", "title", "meta", "badge"], "sidebar row")?;
        let id = required_string(&map, "id", "sidebar row id")?;
        if id.is_empty() || id.len() > 96 || id.chars().any(char::is_control) {
            return Err(CallError::invalid("sidebar row id must be 1-96 printable bytes"));
        }
        if !ids.insert(id.clone()) {
            return Err(CallError::invalid(format!("sidebar row id \"{id}\" appears twice")));
        }
        let title = checked_line(&required_string(&map, "title", "sidebar title")?, "sidebar title", limits)?;
        if title.is_empty() {
            return Err(CallError::invalid(format!("sidebar row \"{id}\" has an empty title")));
        }
        add_payload(&mut payload, title.len(), limits)?;
        let meta = optional_line(&map, "meta", "sidebar meta", limits, &mut payload)?;
        let badge = optional_line(&map, "badge", "sidebar badge", limits, &mut payload)?;
        rows.push(SidebarRow { id, title, meta, badge });
    }
    Ok(rows)
}

fn status_reply(value: Dynamic, limits: &Limits) -> Result<Vec<StatusItem>, CallError> {
    let array = as_array(&value).ok_or_else(|| CallError::invalid("status reply must be an array"))?;
    if array.len() > limits.max_entries_per_call {
        return Err(CallError::invalid("status reply has too many items"));
    }
    let mut payload = 0;
    let mut items = Vec::with_capacity(array.len());
    for item in array {
        let map = as_map(&item).ok_or_else(|| CallError::invalid("status item is not a map"))?;
        reject_unknown(&map, &["text"], "status item")?;
        let text = checked_line(&required_string(&map, "text", "status text")?, "status text", limits)?;
        add_payload(&mut payload, text.len(), limits)?;
        items.push(StatusItem { text });
    }
    Ok(items)
}

fn info_reply(value: Dynamic, limits: &Limits) -> Result<Vec<InfoRow>, CallError> {
    let array = as_array(&value).ok_or_else(|| CallError::invalid("info reply must be an array"))?;
    if array.len() > limits.max_entries_per_call {
        return Err(CallError::invalid("info reply has too many rows"));
    }
    let mut payload = 0;
    let mut rows = Vec::with_capacity(array.len());
    for item in array {
        let map = as_map(&item).ok_or_else(|| CallError::invalid("info row is not a map"))?;
        reject_unknown(&map, &["label", "value"], "info row")?;
        let label = checked_line(&required_string(&map, "label", "info label")?, "info label", limits)?;
        let value = checked_line(&required_string(&map, "value", "info value")?, "info value", limits)?;
        add_payload(&mut payload, label.len() + value.len(), limits)?;
        rows.push(InfoRow { label, value });
    }
    Ok(rows)
}

fn icon_pack_reply(value: Dynamic, limits: &Limits) -> Result<IconPack, CallError> {
    let map = as_map(&value).ok_or_else(|| CallError::invalid("icon-pack reply must be a map"))?;
    reject_unknown(&map, &["icons"], "icon-pack reply")?;
    let icons_value = map.get("icons").ok_or_else(|| CallError::invalid("icon-pack reply has no icons map"))?;
    let icons = as_map(icons_value).ok_or_else(|| CallError::invalid("icons must be a map"))?;
    if icons.len() > 1024 {
        return Err(CallError::invalid("icon pack has more than 1024 icons"));
    }
    let mut payload = 0;
    let mut out = BTreeMap::new();
    for (key, value) in icons {
        if key.is_empty() || key.len() > 256 || key.chars().any(char::is_control) {
            return Err(CallError::invalid("icon key must be 1-256 printable bytes"));
        }
        let mapping = as_map(&value).ok_or_else(|| CallError::invalid(format!("icon \"{key}\" is not a map")))?;
        reject_unknown(&mapping, &["icon", "color"], "icon mapping")?;
        let icon = checked_line(&required_string(&mapping, "icon", "icon id")?, "icon id", limits)?;
        let color = optional_line(&mapping, "color", "icon color", limits, &mut payload)?;
        if let Some(color) = &color
            && !valid_icon_color(color)
        {
            return Err(CallError::invalid(format!("icon color \"{color}\" is not a theme role or palette hue")));
        }
        add_payload(&mut payload, icon.len(), limits)?;
        out.insert(key.to_string(), IconMapping { icon, color });
    }
    Ok(IconPack { icons: out })
}

fn valid_icon_color(color: &str) -> bool {
    const ROLES: [&str; 16] = [
        "bg",
        "bg-inset",
        "surface",
        "surface-raised",
        "border",
        "selection",
        "fg",
        "fg-muted",
        "fg-subtle",
        "accent",
        "on-accent",
        "link",
        "info",
        "success",
        "warning",
        "error",
    ];
    ROLES.contains(&color)
        || matches!(
            color,
            "base08"
                | "base09"
                | "base0A"
                | "base0B"
                | "base0C"
                | "base0D"
                | "base0E"
                | "base0F"
                | "base12"
                | "base13"
                | "base14"
                | "base15"
                | "base16"
                | "base17"
        )
}

fn checked_function(name: &str, ast: &AST) -> Result<String, CallError> {
    if !is_identifier(name) {
        return Err(CallError::invalid(format!("\"{name}\" is not a function name")));
    }
    if !ast.iter_functions().any(|function| function.name == name && function.params.len() == 1) {
        return Err(CallError::invalid(format!("function \"{name}\" is not defined with one parameter")));
    }
    Ok(name.to_owned())
}

fn is_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(byte) if byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn required_handle(map: &Map, key: &str) -> Result<u32, CallError> {
    let value = map.get(key).ok_or_else(|| CallError::invalid(format!("reply has no {key}")))?;
    let number =
        value.clone().try_cast::<i64>().ok_or_else(|| CallError::invalid(format!("{key} must be an integer")))?;
    u32::try_from(number).map_err(|_| CallError::invalid(format!("{key} {number} is out of range")))
}

fn required_string(map: &Map, key: &str, what: &str) -> Result<String, CallError> {
    let value = map.get(key).ok_or_else(|| CallError::invalid(format!("{what} is missing")))?;
    value_string(value, what)
}

fn value_string(value: &Dynamic, what: &str) -> Result<String, CallError> {
    value.clone().into_string().map_err(|_| CallError::invalid(format!("{what} must be a string")))
}

fn optional_line(
    map: &Map,
    key: &str,
    what: &str,
    limits: &Limits,
    payload: &mut usize,
) -> Result<Option<String>, CallError> {
    let Some(value) = map.get(key) else {
        return Ok(None);
    };
    let text = checked_line(&value_string(value, what)?, what, limits)?;
    add_payload(payload, text.len(), limits)?;
    Ok(Some(text))
}

fn checked_line(text: &str, what: &str, limits: &Limits) -> Result<String, CallError> {
    if text.len() > limits.max_display_bytes || text.chars().any(char::is_control) {
        return Err(CallError::invalid(format!("{what} is not bounded single-line text")));
    }
    Ok(text.to_owned())
}

fn checked_text(text: &str, what: &str, limits: &Limits) -> Result<String, CallError> {
    if text.len() > limits.max_result_bytes
        || text.chars().any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return Err(CallError::invalid(format!("{what} is not bounded plain text")));
    }
    Ok(text.to_owned())
}

fn add_payload(total: &mut usize, bytes: usize, limits: &Limits) -> Result<(), CallError> {
    *total = total.saturating_add(bytes).saturating_add(8);
    if *total > limits.max_result_bytes {
        return Err(CallError::invalid(format!("reply payload is larger than {} bytes", limits.max_result_bytes)));
    }
    Ok(())
}

fn reject_unknown(map: &Map, allowed: &[&str], what: &str) -> Result<(), CallError> {
    for key in map.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(CallError::invalid(format!("{what} has unknown field \"{key}\"")));
        }
    }
    Ok(())
}

fn as_array(value: &Dynamic) -> Option<Array> {
    value.clone().try_cast::<Array>()
}

fn as_map(value: &Dynamic) -> Option<Map> {
    value.clone().try_cast::<Map>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{EntryRef, RequestInput};
    use crate::manifest::parse_manifest;
    use crow_core::{EntryData, Snapshot};

    fn limits() -> Limits {
        Limits::default()
    }

    fn manifest() -> Manifest {
        parse_manifest(
            r#"
id = "example.test"
name = "Test"
version = "0.1.0"
api = 1
entry = "main.rhai"
provides = ["column", "command", "hook", "icon-pack"]
permissions = { fs = "none" }
"#,
            &limits(),
        )
        .unwrap()
    }

    fn plugin() -> PluginId {
        PluginId::parse("example.test").unwrap()
    }

    fn input() -> RequestInput {
        RequestInput {
            snapshot: Snapshot {
                scope: "/tmp".into(),
                selection: Some(EntryHandle(1)),
                entries: vec![
                    EntryData {
                        handle: EntryHandle(1),
                        name: "a.txt".into(),
                        is_dir: false,
                        size: 1,
                        kind: "Text".into(),
                    },
                    EntryData {
                        handle: EntryHandle(2),
                        name: "b.txt".into(),
                        is_dir: false,
                        size: 2,
                        kind: "Text".into(),
                    },
                ],
            },
            reads: BTreeMap::from([
                (EntryHandle(1), EntryRef::new("/tmp/a.txt")),
                (EntryHandle(2), EntryRef::new("/tmp/b.txt")),
            ]),
        }
    }

    fn registration(source: &str) -> Result<Vec<Descriptor>, CallError> {
        let engine = rhai::Engine::new();
        let ast = engine.compile(source).unwrap();
        let value = engine.call_fn::<Dynamic>(&mut rhai::Scope::new(), &ast, "register", ()).unwrap();
        validate_registration(&plugin(), &manifest(), &ast, value, &limits())
    }

    #[test]
    fn valid_registration_is_namespaced_and_checked() {
        let descriptors = registration(
            r#"
fn register() {
    [
        #{ kind: "command", id: "relative", title: "Copy Relative Path", applies_to: "selection", function: "run" },
        #{ kind: "icon-pack", id: "icons", title: "Icons" }
    ]
}
fn run(ctx) { #{ action: "copy-text", text: "x" } }
"#,
        )
        .unwrap();
        assert_eq!(descriptors.len(), 2);
        assert_eq!(descriptors[0].id.to_string(), "example.test.relative");
        assert_eq!(descriptors[0].kind, ExtensionKind::Command);
        assert_eq!(descriptors[1].kind, ExtensionKind::IconPack);
        assert!(descriptors[1].function.is_none());
    }

    #[test]
    fn duplicate_unknown_and_undeclared_descriptors_are_rejected() {
        let duplicate = registration(
            r#"
fn register() {
    [
        #{ kind: "command", id: "x", title: "X", function: "run" },
        #{ kind: "command", id: "x", title: "X2", function: "run" }
    ]
}
fn run(ctx) { #{} }
"#,
        );
        assert_eq!(duplicate.unwrap_err().kind, crate::dto::CallErrorKind::InvalidReply);

        let unknown = registration(
            r#"
fn register() { [ #{ kind: "command", id: "x", title: "X", function: "run", colour: "red" } ] }
fn run(ctx) { #{} }
"#,
        );
        assert_eq!(unknown.unwrap_err().kind, crate::dto::CallErrorKind::InvalidReply);

        let undeclared = registration(
            r#"
fn register() { [ #{ kind: "sidebar", id: "x", title: "X", function: "run" } ] }
fn run(ctx) { [] }
"#,
        );
        assert_eq!(undeclared.unwrap_err().kind, crate::dto::CallErrorKind::InvalidReply);

        let missing =
            registration(r#"fn register() { [ #{ kind: "command", id: "x", title: "X", function: "nope" } ] }"#);
        assert_eq!(missing.unwrap_err().kind, crate::dto::CallErrorKind::InvalidReply);
    }

    #[test]
    fn reply_shapes_are_closed_and_keyed_by_input_handles() {
        let manifest = manifest();
        let descriptor = Descriptor {
            kind: ExtensionKind::Column,
            id: ExtensionId::new(plugin(), LocalId::parse("lines").unwrap()),
            title: "Lines".into(),
            applies_to: None,
            function: Some("run".into()),
        };
        let value: Dynamic = rhai::Engine::new()
            .eval::<Dynamic>(r#"[ #{ handle: 1, text: "3" }, #{ handle: 2, text: "4", spoken: "four" } ]"#)
            .unwrap();
        match reply_for(&descriptor, value, &input(), &limits()).unwrap() {
            Reply::Values(ContributionValues::Column(values)) => {
                assert_eq!(values.len(), 2);
                assert_eq!(values[1].spoken.as_deref(), Some("four"));
            }
            other => panic!("{other:?}"),
        }

        let unknown: Dynamic = rhai::Engine::new().eval::<Dynamic>(r#"[ #{ handle: 9, text: "x" } ]"#).unwrap();
        assert_eq!(
            reply_for(&descriptor, unknown, &input(), &limits()).unwrap_err().kind,
            crate::dto::CallErrorKind::InvalidReply
        );
        let duplicate: Dynamic =
            rhai::Engine::new().eval::<Dynamic>(r#"[ #{ handle: 1, text: "x" }, #{ handle: 1, text: "y" } ]"#).unwrap();
        assert!(reply_for(&descriptor, duplicate, &input(), &limits()).is_err());
        assert_eq!(manifest.provides.len(), 4);
    }

    #[test]
    fn command_and_hook_replies_are_typed() {
        let command = Descriptor {
            kind: ExtensionKind::Command,
            id: ExtensionId::new(plugin(), LocalId::parse("copy").unwrap()),
            title: "Copy".into(),
            applies_to: None,
            function: Some("run".into()),
        };
        let value: Dynamic = rhai::Engine::new().eval::<Dynamic>(r#"#{ action: "copy-text", text: "hello" }"#).unwrap();
        assert_eq!(
            reply_for(&command, value, &input(), &limits()).unwrap(),
            Reply::Intent(CommandIntent::CopyText("hello".into()))
        );
        let bad: Dynamic = rhai::Engine::new().eval::<Dynamic>(r#"#{ action: "open-url", text: "x" }"#).unwrap();
        assert!(reply_for(&command, bad, &input(), &limits()).is_err());

        let hook = Descriptor {
            kind: ExtensionKind::Hook,
            id: ExtensionId::new(plugin(), LocalId::parse("hook").unwrap()),
            title: "Hook".into(),
            applies_to: None,
            function: Some("run".into()),
        };
        let refresh: Dynamic = rhai::Engine::new().eval::<Dynamic>(r#"#{ refresh: true }"#).unwrap();
        assert_eq!(reply_for(&hook, refresh, &input(), &limits()).unwrap(), Reply::Refresh);
        let unit: Dynamic = rhai::Engine::new().eval::<Dynamic>(r#"()"#).unwrap();
        assert_eq!(reply_for(&hook, unit, &input(), &limits()).unwrap(), Reply::Unit);
    }

    #[test]
    fn oversized_nested_tables_are_rejected_before_the_host_sees_them() {
        let descriptor = Descriptor {
            kind: ExtensionKind::Preview,
            id: ExtensionId::new(plugin(), LocalId::parse("preview").unwrap()),
            title: "Preview".into(),
            applies_to: None,
            function: Some("run".into()),
        };
        let rows = vec!["[\"x\"]"; 129].join(",");
        let value: Dynamic = rhai::Engine::new().eval::<Dynamic>(&format!("#{{ table: [{rows}] }}")).unwrap();
        assert_eq!(
            reply_for(&descriptor, value, &input(), &limits()).unwrap_err().kind,
            crate::dto::CallErrorKind::InvalidReply
        );
    }

    #[test]
    fn oversized_replies_fail_before_the_host_sees_them() {
        let mut limits = limits();
        limits.max_display_bytes = 8;
        let descriptor = Descriptor {
            kind: ExtensionKind::Column,
            id: ExtensionId::new(plugin(), LocalId::parse("lines").unwrap()),
            title: "Lines".into(),
            applies_to: None,
            function: Some("run".into()),
        };
        let value: Dynamic = rhai::Engine::new().eval::<Dynamic>(r#"[ #{ handle: 1, text: "0123456789" } ]"#).unwrap();
        assert!(reply_for(&descriptor, value, &input(), &limits).is_err());

        let limits = Limits { max_result_bytes: 16, ..Limits::default() };
        let value: Dynamic =
            rhai::Engine::new().eval::<Dynamic>(r#"[ #{ handle: 1, text: "long enough to exceed" } ]"#).unwrap();
        assert!(reply_for(&descriptor, value, &input(), &limits).is_err());
    }
}
