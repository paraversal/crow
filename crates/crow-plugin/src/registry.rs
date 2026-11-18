//! Registration and reply validation. One malformed descriptor or field rejects
//! the whole batch; nothing partial reaches the host.

use std::collections::HashSet;

use crow_core::{
    CommandIntent, ContributionValues, Descriptor, DescriptorSource, EntryHandle, EntryValue, ExtensionId,
    ExtensionKind, InfoRow, LocalId, PluginId, PluginPreview, SidebarRow, StatusItem,
};
use rhai::{AST, Array, Dynamic, Map};

use crate::dto::{CallError, Reply, RequestInput};
use crate::limits::Limits;
use crate::manifest::{Manifest, validate_relative};
use crate::package::PackageSnapshot;

pub fn validate_registration(
    plugin: &PluginId,
    manifest: &Manifest,
    snapshot: &PackageSnapshot,
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
        reject_unknown(&map, &["kind", "id", "title", "applies_to", "function", "asset"], "descriptor")?;
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
        let asset = match map.get("asset") {
            None => None,
            Some(value) => Some(value_string(value, "descriptor asset")?),
        };
        let source = match (kind, function, asset) {
            (ExtensionKind::IconPack, Some(_), _) => {
                return Err(CallError::invalid(format!(
                    "icon-pack \"{local}\" is data-only and cannot name a function"
                )));
            }
            (ExtensionKind::IconPack, None, None) => {
                return Err(CallError::invalid(format!("icon-pack \"{local}\" needs a declared asset")));
            }
            (ExtensionKind::IconPack, None, Some(asset)) => {
                DescriptorSource::Asset(checked_pack_asset(&asset, &local, manifest, snapshot)?)
            }
            (_, _, Some(_)) => {
                return Err(CallError::invalid(format!(
                    "descriptor \"{local}\" can name an asset only when kind is icon-pack"
                )));
            }
            (_, Some(function), None) => DescriptorSource::Function(function),
            (_, None, None) => {
                return Err(CallError::invalid(format!("descriptor \"{local}\" needs a named function")));
            }
        };
        descriptors.push(Descriptor { kind, id: ExtensionId::new(plugin.clone(), local), title, applies_to, source });
    }
    Ok(descriptors)
}

/// An icon-pack asset must be a manifest-declared bundle file and present in
/// the exact approved snapshot. A path that only exists on disk now is not
/// enough.
fn checked_pack_asset(
    asset: &str,
    local: &LocalId,
    manifest: &Manifest,
    snapshot: &PackageSnapshot,
) -> Result<String, CallError> {
    validate_relative(asset).map_err(|error| CallError::invalid(format!("icon-pack \"{local}\" asset: {error}")))?;
    if !manifest.assets.iter().any(|declared| declared == asset) {
        return Err(CallError::invalid(format!(
            "icon-pack \"{local}\" asset \"{asset}\" is not declared in plugin.toml assets"
        )));
    }
    if !snapshot.files.contains_key(asset) {
        return Err(CallError::invalid(format!(
            "icon-pack \"{local}\" asset \"{asset}\" is missing from the approved snapshot"
        )));
    }
    Ok(asset.to_owned())
}

/// Host-side defence in depth after the worker returns descriptors.
pub fn validate_published(
    plugin: &PluginId,
    snapshot: &PackageSnapshot,
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
        if !snapshot.manifest.provides.contains(&descriptor.kind) {
            return Err(format!("descriptor {} is not declared in provides", descriptor.id));
        }
        if !locals.insert(descriptor.id.local.clone()) {
            return Err(format!("descriptor {} is registered twice", descriptor.id));
        }
        if descriptor.title.is_empty() || descriptor.title.len() > limits.max_display_bytes {
            return Err(format!("descriptor {} has an invalid title", descriptor.id));
        }
        match (descriptor.kind, &descriptor.source) {
            (ExtensionKind::IconPack, DescriptorSource::Asset(asset)) => {
                checked_pack_asset(asset, &descriptor.id.local, &snapshot.manifest, snapshot)
                    .map_err(|error| format!("descriptor {}: {}", descriptor.id, error.message))?;
            }
            (ExtensionKind::IconPack, DescriptorSource::Function(_)) => {
                return Err(format!("descriptor {} is an icon pack with a function", descriptor.id));
            }
            (_, DescriptorSource::Asset(_)) => {
                return Err(format!("descriptor {} is not an icon pack but names an asset", descriptor.id));
            }
            (_, DescriptorSource::Function(function)) => {
                if function.is_empty() {
                    return Err(format!("descriptor {} has no function", descriptor.id));
                }
            }
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
        ExtensionKind::IconPack => {
            Err(CallError::invalid("icon-pack is registered from an approved asset, not evaluated"))
        }
    }
}

fn command_reply(value: Dynamic, limits: &Limits) -> Result<CommandIntent, CallError> {
    let map = as_map(&value).ok_or_else(|| CallError::invalid("a command must return a map"))?;
    reject_unknown(&map, &["kind", "text"], "command reply")?;
    let kind = required_string(&map, "kind", "command kind")?;
    if kind != "copy_text" {
        return Err(CallError::invalid(format!("command kind \"{kind}\" is not in the api 1 allowlist")));
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
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;
    use crate::dto::RequestInput;
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
assets = ["icons/pack.toml", "icons/rust.svg"]
provides = ["column", "command", "hook", "icon-pack"]
permissions = { fs = "none" }
"#,
            &limits(),
        )
        .unwrap()
    }

    fn snapshot() -> PackageSnapshot {
        let manifest = manifest();
        let files = [
            ("plugin.toml", "manifest".as_bytes()),
            ("main.rhai", "fn register() { [] }".as_bytes()),
            ("icons/pack.toml", "api = 1".as_bytes()),
            ("icons/rust.svg", "<svg/>".as_bytes()),
        ]
        .into_iter()
        .map(|(name, bytes)| (name.to_owned(), Arc::from(bytes)))
        .collect::<BTreeMap<_, _>>();
        PackageSnapshot { manifest, files, digest: "0".repeat(64) }
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
            excerpt: None,
        }
    }

    fn registration(source: &str) -> Result<Vec<Descriptor>, CallError> {
        let engine = rhai::Engine::new();
        let ast = engine.compile(source).unwrap();
        let value = engine.call_fn::<Dynamic>(&mut rhai::Scope::new(), &ast, "register", ()).unwrap();
        let snapshot = snapshot();
        validate_registration(&plugin(), &snapshot.manifest, &snapshot, &ast, value, &limits())
    }

    #[test]
    fn valid_registration_is_namespaced_and_checked() {
        let descriptors = registration(
            r#"
fn register() {
    [
        #{ kind: "command", id: "relative", title: "Copy Relative Path", applies_to: "selection", function: "run" },
        #{ kind: "icon-pack", id: "icons", title: "Icons", asset: "icons/pack.toml" }
    ]
}
fn run(ctx) { #{ kind: "copy_text", text: "x" } }
"#,
        )
        .unwrap();
        assert_eq!(descriptors.len(), 2);
        assert_eq!(descriptors[0].id.to_string(), "example.test.relative");
        assert_eq!(descriptors[0].kind, ExtensionKind::Command);
        assert_eq!(descriptors[1].kind, ExtensionKind::IconPack);
        assert!(descriptors[1].function().is_none());
        assert_eq!(descriptors[1].asset(), Some("icons/pack.toml"));
    }

    #[test]
    fn icon_pack_assets_must_be_declared_and_present() {
        let missing = registration(
            r#"
fn register() { [ #{ kind: "icon-pack", id: "icons", title: "Icons" } ] }
"#,
        );
        assert!(missing.unwrap_err().message.contains("needs a declared asset"));

        let undeclared = registration(
            r#"
fn register() { [ #{ kind: "icon-pack", id: "icons", title: "Icons", asset: "icons/other.toml" } ] }
"#,
        );
        assert!(undeclared.unwrap_err().message.contains("not declared"));

        let traversal = registration(
            r#"
fn register() { [ #{ kind: "icon-pack", id: "icons", title: "Icons", asset: "../pack.toml" } ] }
"#,
        );
        assert!(traversal.unwrap_err().message.contains("relative file path"));

        let wrong_kind = registration(
            r#"
fn register() { [ #{ kind: "command", id: "copy", title: "Copy", function: "run", asset: "icons/pack.toml" } ] }
fn run(ctx) { #{ kind: "copy_text", text: "x" } }
"#,
        );
        assert!(wrong_kind.unwrap_err().message.contains("only when kind is icon-pack"));

        let mut snapshot = snapshot();
        snapshot.files.remove("icons/pack.toml");
        let engine = rhai::Engine::new();
        let ast = engine
            .compile(r#"fn register() { [ #{ kind: "icon-pack", id: "icons", title: "Icons", asset: "icons/pack.toml" } ] }"#)
            .unwrap();
        let value = engine.call_fn::<Dynamic>(&mut rhai::Scope::new(), &ast, "register", ()).unwrap();
        assert!(
            validate_registration(&plugin(), &snapshot.manifest, &snapshot, &ast, value, &limits())
                .unwrap_err()
                .message
                .contains("missing from the approved snapshot")
        );
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
            source: DescriptorSource::Function("run".into()),
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
            source: DescriptorSource::Function("run".into()),
        };
        let value: Dynamic = rhai::Engine::new().eval::<Dynamic>(r#"#{ kind: "copy_text", text: "hello" }"#).unwrap();
        assert_eq!(
            reply_for(&command, value, &input(), &limits()).unwrap(),
            Reply::Intent(CommandIntent::CopyText("hello".into()))
        );
        let bad: Dynamic = rhai::Engine::new().eval::<Dynamic>(r#"#{ kind: "open_url", text: "x" }"#).unwrap();
        assert!(reply_for(&command, bad, &input(), &limits()).is_err());

        let hook = Descriptor {
            kind: ExtensionKind::Hook,
            id: ExtensionId::new(plugin(), LocalId::parse("hook").unwrap()),
            title: "Hook".into(),
            applies_to: None,
            source: DescriptorSource::Function("run".into()),
        };
        let refresh: Dynamic = rhai::Engine::new().eval::<Dynamic>(r#"#{ refresh: true }"#).unwrap();
        assert_eq!(reply_for(&hook, refresh, &input(), &limits()).unwrap(), Reply::Refresh);
        let unit: Dynamic = rhai::Engine::new().eval::<Dynamic>(r#"()"#).unwrap();
        assert_eq!(reply_for(&hook, unit, &input(), &limits()).unwrap(), Reply::Unit);
    }

    #[test]
    fn every_other_reply_shape_is_validated() {
        let shape = |kind: ExtensionKind, source: &str| {
            let descriptor = Descriptor {
                kind,
                id: ExtensionId::new(plugin(), LocalId::parse("shape").unwrap()),
                title: "Shape".into(),
                applies_to: None,
                source: if kind == ExtensionKind::IconPack {
                    DescriptorSource::Asset("icons/pack.toml".into())
                } else {
                    DescriptorSource::Function("run".into())
                },
            };
            let value: Dynamic = rhai::Engine::new().eval::<Dynamic>(source).unwrap();
            reply_for(&descriptor, value, &input(), &limits())
        };

        match shape(ExtensionKind::Preview, r#"#{ text: "hello" }"#).unwrap() {
            Reply::Values(ContributionValues::Preview(preview)) => assert_eq!(preview.text.as_deref(), Some("hello")),
            other => panic!("{other:?}"),
        }
        match shape(ExtensionKind::Sidebar, r#"[ #{ id: "a", title: "A" } ]"#).unwrap() {
            Reply::Values(ContributionValues::Sidebar(rows)) => assert_eq!(rows[0].title, "A"),
            other => panic!("{other:?}"),
        }
        match shape(ExtensionKind::Status, r#"[ #{ text: "S" } ]"#).unwrap() {
            Reply::Values(ContributionValues::Status(items)) => assert_eq!(items[0].text, "S"),
            other => panic!("{other:?}"),
        }
        match shape(ExtensionKind::Info, r#"[ #{ label: "L", value: "V" } ]"#).unwrap() {
            Reply::Values(ContributionValues::Info(rows)) => assert_eq!(rows[0].value, "V"),
            other => panic!("{other:?}"),
        }
        assert!(
            shape(ExtensionKind::IconPack, r#"#{ icons: #{} }"#).unwrap_err().message.contains("approved asset"),
            "an icon pack is registered from an asset, never evaluated"
        );

        assert!(shape(ExtensionKind::Sidebar, r#"[ #{ id: "a", title: "A", extra: 1 } ]"#).is_err());
        assert!(shape(ExtensionKind::Preview, r#"#{ text: "x", table: [] }"#).is_err());
    }

    #[test]
    fn oversized_nested_tables_are_rejected_before_the_host_sees_them() {
        let descriptor = Descriptor {
            kind: ExtensionKind::Preview,
            id: ExtensionId::new(plugin(), LocalId::parse("preview").unwrap()),
            title: "Preview".into(),
            applies_to: None,
            source: DescriptorSource::Function("run".into()),
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
            source: DescriptorSource::Function("run".into()),
        };
        let value: Dynamic = rhai::Engine::new().eval::<Dynamic>(r#"[ #{ handle: 1, text: "0123456789" } ]"#).unwrap();
        assert!(reply_for(&descriptor, value, &input(), &limits).is_err());

        let limits = Limits { max_result_bytes: 16, ..Limits::default() };
        let value: Dynamic =
            rhai::Engine::new().eval::<Dynamic>(r#"[ #{ handle: 1, text: "long enough to exceed" } ]"#).unwrap();
        assert!(reply_for(&descriptor, value, &input(), &limits).is_err());
    }
}
