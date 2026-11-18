//! Registration and reply validation. One malformed descriptor or field rejects
//! the whole batch; nothing partial reaches the host.

use std::collections::HashSet;

use crow_core::{
    Align, CommandIntent, CommandTarget, ContributionValues, Descriptor, DescriptorSource, EntryHandle, EntryValue,
    ExtensionId, ExtensionKind, InfoRow, LocalId, PluginId, PluginPreview, SidebarRow, StatusItem, Tint,
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
        let kind_key = required_string(&map, "kind", "descriptor kind")?;
        let kind = ExtensionKind::parse(&kind_key)
            .ok_or_else(|| CallError::invalid(format!("descriptor {index} has unknown kind \"{kind_key}\"")))?;
        let common = ["kind", "id", "title", "function", "asset"];
        let allowed: &[&str] = match kind {
            ExtensionKind::Column => &["kind", "id", "title", "function", "width", "min_width", "align"],
            ExtensionKind::Preview => &["kind", "id", "title", "function", "extensions"],
            ExtensionKind::Command => &["kind", "id", "title", "function", "applies_to", "context_menu", "extensions"],
            _ => &common,
        };
        reject_unknown(&map, allowed, "descriptor")?;
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
        let mut descriptor = Descriptor::new(kind, ExtensionId::new(plugin.clone(), local.clone()), title, source);
        match kind {
            ExtensionKind::Column => {
                let width = optional_u16(&map, "width", "descriptor width")?.unwrap_or(100);
                let min_width = optional_u16(&map, "min_width", "descriptor min_width")?.unwrap_or(50);
                if !(32..=480).contains(&width) || !(32..=480).contains(&min_width) {
                    return Err(CallError::invalid(format!(
                        "column \"{local}\" needs 32 <= min_width <= width <= 480"
                    )));
                }
                if min_width > width {
                    return Err(CallError::invalid(format!(
                        "column \"{local}\" needs 32 <= min_width <= width <= 480"
                    )));
                }
                let align = match map.get("align") {
                    None => Align::Left,
                    Some(value) => match value_string(value, "descriptor align")?.as_str() {
                        "left" => Align::Left,
                        "right" => Align::Right,
                        other => {
                            return Err(CallError::invalid(format!(
                                "column \"{local}\" align \"{other}\" is not left or right"
                            )));
                        }
                    },
                };
                descriptor.width = width;
                descriptor.min_width = min_width;
                descriptor.align = align;
            }
            ExtensionKind::Preview => {
                descriptor.extensions = extensions_field(&map, "extensions", limits)?;
            }
            ExtensionKind::Command => {
                let applies_to = match map.get("applies_to") {
                    None => CommandTarget::Scope,
                    Some(value) => {
                        let text = value_string(value, "descriptor applies_to")?;
                        CommandTarget::parse(&text).ok_or_else(|| {
                            CallError::invalid(format!(
                                "command \"{local}\" applies_to must be \"scope\" or \"selection\""
                            ))
                        })?
                    }
                };
                let context_menu = optional_bool(&map, "context_menu", "descriptor context_menu")?.unwrap_or(false);
                if context_menu && applies_to != CommandTarget::Selection {
                    return Err(CallError::invalid(format!(
                        "command \"{local}\" can use context_menu only with applies_to = \"selection\""
                    )));
                }
                let extensions = extensions_field(&map, "extensions", limits)?;
                if extensions.is_some() && applies_to != CommandTarget::Selection {
                    return Err(CallError::invalid(format!(
                        "command \"{local}\" can use extensions only with applies_to = \"selection\""
                    )));
                }
                descriptor.applies_to = applies_to;
                descriptor.context_menu = context_menu;
                descriptor.extensions = extensions;
            }
            _ => {}
        }
        descriptors.push(descriptor);
    }
    Ok(descriptors)
}

fn optional_u16(map: &Map, key: &str, what: &str) -> Result<Option<u16>, CallError> {
    let Some(value) = map.get(key) else { return Ok(None) };
    let number =
        value.clone().try_cast::<i64>().ok_or_else(|| CallError::invalid(format!("{what} must be an integer")))?;
    u16::try_from(number).map(Some).map_err(|_| CallError::invalid(format!("{what} {number} is out of range")))
}

fn optional_bool(map: &Map, key: &str, what: &str) -> Result<Option<bool>, CallError> {
    let Some(value) = map.get(key) else { return Ok(None) };
    value.clone().try_cast::<bool>().map(Some).ok_or_else(|| CallError::invalid(format!("{what} must be a bool")))
}

/// A preview/command suffix list: 1-32 unique ASCII-folded suffixes, each 1-32
/// letters, digits, hyphens or underscores without a leading dot.
fn extensions_field(map: &Map, key: &str, limits: &Limits) -> Result<Option<Vec<String>>, CallError> {
    let Some(value) = map.get(key) else { return Ok(None) };
    let array = as_array(value).ok_or_else(|| CallError::invalid("descriptor extensions must be an array"))?;
    if array.is_empty() {
        return Err(CallError::invalid("descriptor extensions must name at least one suffix"));
    }
    if array.len() > limits.max_extensions {
        return Err(CallError::invalid(format!(
            "descriptor extensions has more than {} suffixes",
            limits.max_extensions
        )));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(array.len());
    for item in array {
        let raw = value_string(&item, "descriptor suffix")?;
        let folded = fold_suffix(&raw, limits)?;
        if !seen.insert(folded.clone()) {
            return Err(CallError::invalid(format!("descriptor suffix \"{raw}\" appears twice")));
        }
        out.push(folded);
    }
    Ok(Some(out))
}

fn fold_suffix(raw: &str, limits: &Limits) -> Result<String, CallError> {
    if raw.is_empty() || raw.len() > limits.max_extension_bytes {
        return Err(CallError::invalid(format!("suffix \"{raw}\" must be 1-{} bytes", limits.max_extension_bytes)));
    }
    if raw.starts_with('.') {
        return Err(CallError::invalid(format!("suffix \"{raw}\" must not start with a dot")));
    }
    if !raw.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_') {
        return Err(CallError::invalid(format!(
            "suffix \"{raw}\" must use ASCII letters, digits, hyphens or underscores"
        )));
    }
    Ok(raw.to_ascii_lowercase())
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
        if !(32..=480).contains(&descriptor.width)
            || !(32..=480).contains(&descriptor.min_width)
            || descriptor.min_width > descriptor.width
        {
            return Err(format!("descriptor {} has invalid column bounds", descriptor.id));
        }
        if descriptor.context_menu && descriptor.applies_to != CommandTarget::Selection {
            return Err(format!("descriptor {} has context_menu without a selection target", descriptor.id));
        }
        if let Some(extensions) = &descriptor.extensions {
            let allowed = match descriptor.kind {
                ExtensionKind::Preview => true,
                ExtensionKind::Command => descriptor.applies_to == CommandTarget::Selection,
                _ => false,
            };
            if !allowed {
                return Err(format!("descriptor {} cannot declare extensions", descriptor.id));
            }
            if extensions.is_empty() || extensions.len() > limits.max_extensions {
                return Err(format!("descriptor {} has an invalid extension count", descriptor.id));
            }
            let mut seen = HashSet::new();
            for suffix in extensions {
                if fold_suffix(suffix, limits).map(|folded| folded != *suffix).unwrap_or(true) {
                    return Err(format!("descriptor {} has an invalid extension", descriptor.id));
                }
                if !seen.insert(suffix) {
                    return Err(format!("descriptor {} repeats an extension", descriptor.id));
                }
            }
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
        ExtensionKind::Command => command_reply(value, input, limits),
        ExtensionKind::Hook => Ok(hook_reply(value)?),
        ExtensionKind::Column | ExtensionKind::Decoration => {
            let values = entry_values(value, input, limits, descriptor.kind)?;
            Ok(Reply::Values(if descriptor.kind == ExtensionKind::Column {
                ContributionValues::Column(values)
            } else {
                ContributionValues::Decoration(values)
            }))
        }
        ExtensionKind::Preview => Ok(Reply::Values(ContributionValues::Preview(preview_reply(value, limits)?))),
        ExtensionKind::Sidebar => Ok(Reply::Values(ContributionValues::Sidebar(sidebar_reply(value, input, limits)?))),
        ExtensionKind::Status => Ok(Reply::Values(ContributionValues::Status(status_reply(value, limits)?))),
        ExtensionKind::Info => Ok(Reply::Values(ContributionValues::Info(info_reply(value, limits)?))),
        ExtensionKind::IconPack => {
            Err(CallError::invalid("icon-pack is registered from an approved asset, not evaluated"))
        }
    }
}

fn command_reply(value: Dynamic, input: &RequestInput, limits: &Limits) -> Result<Reply, CallError> {
    if value.is_unit() {
        return Ok(Reply::Unit);
    }
    let map = as_map(&value).ok_or_else(|| CallError::invalid("a command must return unit or a map"))?;
    reject_unknown(&map, &["kind", "text", "handle"], "command reply")?;
    let kind = required_string(&map, "kind", "command kind")?;
    match kind.as_str() {
        "message" => {
            reject_unknown(&map, &["kind", "text"], "message reply")?;
            let text = checked_line(&required_string(&map, "text", "command text")?, "command text", limits)?;
            if text.is_empty() {
                return Err(CallError::invalid("a message must not be empty"));
            }
            Ok(Reply::Intent(CommandIntent::Message(text)))
        }
        "copy_text" => {
            reject_unknown(&map, &["kind", "text"], "copy_text reply")?;
            let text = checked_text(&required_string(&map, "text", "command text")?, "command text", limits)?;
            Ok(Reply::Intent(CommandIntent::CopyText(text)))
        }
        "reveal" => {
            reject_unknown(&map, &["kind", "handle"], "reveal reply")?;
            let handle = EntryHandle(required_handle(&map, "handle")?);
            if !input.snapshot.entries.iter().any(|entry| entry.handle == handle) {
                return Err(CallError::invalid(format!("reveal names unknown entry handle {}", handle.0)));
            }
            Ok(Reply::Intent(CommandIntent::Reveal(handle)))
        }
        other => Err(CallError::invalid(format!("command kind \"{other}\" is not in the api 1 allowlist"))),
    }
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

fn entry_values(
    value: Dynamic,
    input: &RequestInput,
    limits: &Limits,
    kind: ExtensionKind,
) -> Result<Vec<EntryValue>, CallError> {
    if value.is_unit() {
        return Ok(Vec::new());
    }
    let array = as_array(&value).ok_or_else(|| CallError::invalid("column/decor reply must be an array"))?;
    if array.len() > limits.max_entries_per_call {
        return Err(CallError::invalid(format!(
            "column/decor reply has {} entries; the limit is {}",
            array.len(),
            limits.max_entries_per_call
        )));
    }
    let decoration = kind == ExtensionKind::Decoration;
    let label = if decoration { "decoration" } else { "column" };
    let known: HashSet<EntryHandle> = input.snapshot.entries.iter().map(|entry| entry.handle).collect();
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(array.len());
    let mut payload = 0;
    for item in array {
        let map = as_map(&item).ok_or_else(|| CallError::invalid(format!("{label} value is not a map")))?;
        let allowed: &[&str] =
            if decoration { &["handle", "text", "spoken", "tint", "dim"] } else { &["handle", "text", "spoken"] };
        reject_unknown(&map, allowed, &format!("{label} value"))?;
        let Some(handle_value) = map.get("handle") else {
            if decoration {
                // An omitted handle contributes nothing.
                continue;
            }
            return Err(CallError::invalid("column value has no handle"));
        };
        let handle = EntryHandle(
            handle_value
                .clone()
                .try_cast::<i64>()
                .and_then(|number| u32::try_from(number).ok())
                .ok_or_else(|| CallError::invalid("handle must be an integer in range"))?,
        );
        if !known.contains(&handle) {
            return Err(CallError::invalid(format!("reply names unknown entry handle {}", handle.0)));
        }
        if !seen.insert(handle) {
            return Err(CallError::invalid(format!("reply names entry handle {} twice", handle.0)));
        }
        let text_limit = if decoration { limits.max_badge_bytes } else { limits.max_display_bytes };
        let text = checked_line_limit(
            &required_string(&map, "text", &format!("{label} text"))?,
            &format!("{label} text"),
            text_limit,
        )?;
        add_payload(&mut payload, text.len() + 8, limits)?;
        let spoken = match map.get("spoken") {
            None => None,
            Some(value) => {
                let spoken = checked_line(
                    &value_string(value, &format!("{label} spoken"))?,
                    &format!("{label} spoken"),
                    limits,
                )?;
                add_payload(&mut payload, spoken.len() + 8, limits)?;
                Some(spoken)
            }
        };
        let tint = match map.get("tint") {
            None => None,
            Some(value) => {
                if !decoration {
                    return Err(CallError::invalid("a column value cannot set tint"));
                }
                let raw = value_string(value, "decoration tint")?;
                Some(
                    Tint::parse(&raw)
                        .ok_or_else(|| CallError::invalid(format!("unknown decoration tint \"{raw}\"")))?,
                )
            }
        };
        let dim = match map.get("dim") {
            None => false,
            Some(value) => {
                if !decoration {
                    return Err(CallError::invalid("a column value cannot set dim"));
                }
                value.clone().try_cast::<bool>().ok_or_else(|| CallError::invalid("decoration dim must be a bool"))?
            }
        };
        out.push(EntryValue { handle, text, spoken, tint, dim });
    }
    Ok(out)
}

fn preview_reply(value: Dynamic, limits: &Limits) -> Result<PluginPreview, CallError> {
    if value.is_unit() {
        return Ok(PluginPreview::default());
    }
    let map = as_map(&value).ok_or_else(|| CallError::invalid("a preview must return unit or a map"))?;
    reject_unknown(&map, &["text", "table"], "preview reply")?;
    if map.contains_key("text") && map.contains_key("table") {
        return Err(CallError::invalid("a preview returns text or a table, not both"));
    }
    let text = match map.get("text") {
        None => None,
        Some(value) => {
            let text = checked_text(&value_string(value, "preview text")?, "preview text", limits)?;
            (!text.is_empty()).then_some(text)
        }
    };
    let table = match map.get("table") {
        None => None,
        Some(value) => {
            let rows = as_array(value).ok_or_else(|| CallError::invalid("preview table must be an array of rows"))?;
            if rows.len() > 128 {
                return Err(CallError::invalid("preview table has more than 128 rows"));
            }
            if rows.is_empty() {
                None
            } else {
                let mut payload = 0;
                let mut cells = 0;
                let mut columns = None;
                let mut out = Vec::with_capacity(rows.len());
                for row in rows {
                    let cells_value =
                        as_array(&row).ok_or_else(|| CallError::invalid("preview row must be an array"))?;
                    if cells_value.len() > 64 {
                        return Err(CallError::invalid("preview row has more than 64 cells"));
                    }
                    let width = cells_value.len();
                    if *columns.get_or_insert(width) != width {
                        return Err(CallError::invalid("preview table is not rectangular"));
                    }
                    let mut row_out = Vec::with_capacity(width);
                    for cell in cells_value {
                        let cell = checked_line(&value_string(&cell, "preview cell")?, "preview cell", limits)?;
                        add_payload(&mut payload, cell.len() + 8, limits)?;
                        cells += 1;
                        row_out.push(cell);
                    }
                    out.push(row_out);
                }
                if cells > 1024 {
                    return Err(CallError::invalid("preview table has more than 1024 cells"));
                }
                if cells == 0 { None } else { Some(out) }
            }
        }
    };
    Ok(PluginPreview { text, table })
}

fn sidebar_reply(value: Dynamic, input: &RequestInput, limits: &Limits) -> Result<Vec<SidebarRow>, CallError> {
    if value.is_unit() {
        return Ok(Vec::new());
    }
    let array = as_array(&value).ok_or_else(|| CallError::invalid("sidebar reply must be an array"))?;
    if array.len() > limits.max_entries_per_call {
        return Err(CallError::invalid("sidebar reply has too many rows"));
    }
    let known: HashSet<EntryHandle> = input.snapshot.entries.iter().map(|entry| entry.handle).collect();
    let mut ids = HashSet::new();
    let mut rows = Vec::with_capacity(array.len());
    let mut payload = 0;
    for item in array {
        let map = as_map(&item).ok_or_else(|| CallError::invalid("sidebar row is not a map"))?;
        reject_unknown(&map, &["id", "title", "meta", "badge", "handle"], "sidebar row")?;
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
        add_payload(&mut payload, title.len() + 8, limits)?;
        let meta = optional_line(&map, "meta", "sidebar meta", limits, &mut payload)?;
        let badge_value = optional_line(&map, "badge", "sidebar badge", limits, &mut payload)?;
        let badge = match badge_value {
            Some(badge) => {
                let badge = checked_line_limit(&badge, "sidebar badge", limits.max_badge_bytes)?;
                Some(badge)
            }
            None => None,
        };
        let handle = match map.get("handle") {
            None => None,
            Some(value) => {
                let handle = EntryHandle(
                    value
                        .clone()
                        .try_cast::<i64>()
                        .and_then(|number| u32::try_from(number).ok())
                        .ok_or_else(|| CallError::invalid("sidebar handle must be an integer in range"))?,
                );
                if !known.contains(&handle) {
                    return Err(CallError::invalid(format!(
                        "sidebar row \"{id}\" names unknown entry handle {}",
                        handle.0
                    )));
                }
                Some(handle)
            }
        };
        rows.push(SidebarRow { id, title, meta, badge, handle });
    }
    Ok(rows)
}

fn status_reply(value: Dynamic, limits: &Limits) -> Result<Vec<StatusItem>, CallError> {
    if value.is_unit() {
        return Ok(Vec::new());
    }
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
    if value.is_unit() {
        return Ok(Vec::new());
    }
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
    checked_line_limit(text, what, limits.max_display_bytes)
}

fn checked_line_limit(text: &str, what: &str, max_bytes: usize) -> Result<String, CallError> {
    if text.len() > max_bytes || text.chars().any(char::is_control) {
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

    fn all_snapshot() -> PackageSnapshot {
        let manifest = parse_manifest(
            r#"
id = "example.test"
name = "Test"
version = "0.1.0"
api = 1
entry = "main.rhai"
assets = ["icons/pack.toml", "icons/rust.svg"]
provides = ["column", "decoration", "preview", "sidebar", "command", "hook", "status", "info", "icon-pack"]
permissions = { fs = "none" }
"#,
            &limits(),
        )
        .unwrap();
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

    fn descriptor(kind: ExtensionKind, local: &str, title: &str, source: DescriptorSource) -> Descriptor {
        Descriptor::new(kind, ExtensionId::new(plugin(), LocalId::parse(local).unwrap()), title.into(), source)
    }

    fn registration(source: &str) -> Result<Vec<Descriptor>, CallError> {
        registration_with(snapshot(), source)
    }

    fn registration_all(source: &str) -> Result<Vec<Descriptor>, CallError> {
        registration_with(all_snapshot(), source)
    }

    fn registration_with(snapshot: PackageSnapshot, source: &str) -> Result<Vec<Descriptor>, CallError> {
        let engine = rhai::Engine::new();
        let ast = engine.compile(source).unwrap();
        let value = engine.call_fn::<Dynamic>(&mut rhai::Scope::new(), &ast, "register", ()).unwrap();
        validate_registration(&plugin(), &snapshot.manifest, &snapshot, &ast, value, &limits())
    }

    fn reply(kind: ExtensionKind, source: &str) -> Result<Reply, CallError> {
        reply_with(kind, source, input(), &limits())
    }

    fn reply_with(kind: ExtensionKind, source: &str, input: RequestInput, limits: &Limits) -> Result<Reply, CallError> {
        let descriptor = descriptor(kind, "shape", "Shape", DescriptorSource::Function("run".into()));
        let value: Dynamic = rhai::Engine::new().eval::<Dynamic>(source).unwrap();
        reply_for(&descriptor, value, &input, limits)
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
        assert!(wrong_kind.unwrap_err().message.contains("unknown field \"asset\""));

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
        let descriptor = descriptor(ExtensionKind::Column, "lines", "Lines", DescriptorSource::Function("run".into()));
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
        let command = descriptor(ExtensionKind::Command, "copy", "Copy", DescriptorSource::Function("run".into()));
        let value: Dynamic = rhai::Engine::new().eval::<Dynamic>(r#"#{ kind: "copy_text", text: "hello" }"#).unwrap();
        assert_eq!(
            reply_for(&command, value, &input(), &limits()).unwrap(),
            Reply::Intent(CommandIntent::CopyText("hello".into()))
        );
        let bad: Dynamic = rhai::Engine::new().eval::<Dynamic>(r#"#{ kind: "open_url", text: "x" }"#).unwrap();
        assert!(reply_for(&command, bad, &input(), &limits()).is_err());

        let hook = descriptor(ExtensionKind::Hook, "hook", "Hook", DescriptorSource::Function("run".into()));
        let refresh: Dynamic = rhai::Engine::new().eval::<Dynamic>(r#"#{ refresh: true }"#).unwrap();
        assert_eq!(reply_for(&hook, refresh, &input(), &limits()).unwrap(), Reply::Refresh);
        let unit: Dynamic = rhai::Engine::new().eval::<Dynamic>(r#"()"#).unwrap();
        assert_eq!(reply_for(&hook, unit, &input(), &limits()).unwrap(), Reply::Unit);
    }

    #[test]
    fn every_other_reply_shape_is_validated() {
        let shape = |kind: ExtensionKind, source: &str| {
            let descriptor = descriptor(
                kind,
                "shape",
                "Shape",
                if kind == ExtensionKind::IconPack {
                    DescriptorSource::Asset("icons/pack.toml".into())
                } else {
                    DescriptorSource::Function("run".into())
                },
            );
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
        let descriptor =
            descriptor(ExtensionKind::Preview, "preview", "Preview", DescriptorSource::Function("run".into()));
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
        let descriptor = descriptor(ExtensionKind::Column, "lines", "Lines", DescriptorSource::Function("run".into()));
        let value: Dynamic = rhai::Engine::new().eval::<Dynamic>(r#"[ #{ handle: 1, text: "0123456789" } ]"#).unwrap();
        assert!(reply_for(&descriptor, value, &input(), &limits).is_err());

        let limits = Limits { max_result_bytes: 16, ..Limits::default() };
        let value: Dynamic =
            rhai::Engine::new().eval::<Dynamic>(r#"[ #{ handle: 1, text: "long enough to exceed" } ]"#).unwrap();
        assert!(reply_for(&descriptor, value, &input(), &limits).is_err());
    }

    fn column_registration(fields: &str) -> String {
        format!(r#"fn register() {{ [ #{{ {fields} }} ] }} fn run(ctx) {{ () }}"#)
    }

    #[test]
    fn column_presentation_defaults_and_bounds() {
        let descriptor =
            registration_all(&column_registration(r#"kind: "column", id: "c", title: "C", function: "run""#))
                .unwrap()
                .remove(0);
        assert_eq!((descriptor.width, descriptor.min_width), (100, 50));
        assert_eq!(descriptor.align, Align::Left);
        assert_eq!(descriptor.applies_to, CommandTarget::Scope);
        assert!(!descriptor.context_menu);
        assert!(descriptor.extensions.is_none());

        let descriptor = registration_all(&column_registration(
            r#"kind: "column", id: "c", title: "C", function: "run", width: 240, min_width: 80, align: "right""#,
        ))
        .unwrap()
        .remove(0);
        assert_eq!((descriptor.width, descriptor.min_width), (240, 80));
        assert_eq!(descriptor.align, Align::Right);

        for fields in [
            r#"kind: "column", id: "c", title: "C", function: "run", width: 31"#,
            r#"kind: "column", id: "c", title: "C", function: "run", width: 481"#,
            r#"kind: "column", id: "c", title: "C", function: "run", width: 64, min_width: 65"#,
            r#"kind: "column", id: "c", title: "C", function: "run", min_width: 31"#,
            r#"kind: "column", id: "c", title: "C", function: "run", width: "100""#,
            r#"kind: "column", id: "c", title: "C", function: "run", align: "center""#,
            r#"kind: "decoration", id: "d", title: "D", function: "run", width: 100"#,
        ] {
            assert!(registration_all(&column_registration(fields)).is_err(), "{fields}");
        }
    }

    #[test]
    fn suffix_filters_are_folded_unique_and_command_only_for_selection() {
        let descriptors = registration_all(
            r#"
fn register() {
    [
        #{ kind: "preview", id: "p", title: "P", function: "run", extensions: ["PNG", "tar-gz", "x_1"] },
        #{ kind: "command", id: "c", title: "C", function: "run", applies_to: "selection",
           context_menu: true, extensions: ["TXT"] }
    ]
}
fn run(ctx) { () }
"#,
        )
        .unwrap();
        assert_eq!(
            descriptors[0].extensions.as_deref(),
            Some(&["png".to_owned(), "tar-gz".to_owned(), "x_1".to_owned()][..])
        );
        assert_eq!(descriptors[1].applies_to, CommandTarget::Selection);
        assert!(descriptors[1].context_menu);
        assert_eq!(descriptors[1].extensions.as_deref(), Some(&["txt".to_owned()][..]));

        let preview = r#"kind: "preview", id: "p", title: "P", function: "run""#;
        assert!(registration_all(&column_registration(&format!(r#"{preview}, extensions: []"#))).is_err());
        assert!(registration_all(&column_registration(&format!(r#"{preview}, extensions: ["PNG", "png"]"#))).is_err());
        assert!(registration_all(&column_registration(&format!(r#"{preview}, extensions: [".txt"]"#))).is_err());
        assert!(registration_all(&column_registration(&format!(r#"{preview}, extensions: ["é"]"#))).is_err());
        assert!(registration_all(&column_registration(&format!(r#"{preview}, extensions: ["x"]"#))).is_ok());
        let too_many = (0..33).map(|i| format!("\"x{i}\"")).collect::<Vec<_>>().join(",");
        assert!(registration_all(&column_registration(&format!(r#"{preview}, extensions: [{too_many}]"#))).is_err());
        let too_long = "x".repeat(33);
        assert!(registration_all(&column_registration(&format!(r#"{preview}, extensions: ["{too_long}"]"#))).is_err());

        let base = r#"kind: "command", id: "c", title: "C", function: "run""#;
        assert!(
            registration_all(&column_registration(&format!(r#"{base}, applies_to: "scope", context_menu: true"#)))
                .is_err()
        );
        assert!(
            registration_all(&column_registration(&format!(r#"{base}, applies_to: "scope", extensions: ["txt"]"#)))
                .is_err()
        );
        assert!(registration_all(&column_registration(&format!(r#"{base}, applies_to: "both""#))).is_err());
        assert!(
            registration_all(&column_registration(
                r#"kind: "preview", id: "p", title: "P", function: "run", applies_to: "selection""#
            ))
            .is_err(),
            "applies_to belongs to commands"
        );
    }

    #[test]
    fn unit_replies_decline_without_error() {
        assert_eq!(reply(ExtensionKind::Column, "()").unwrap(), Reply::Values(ContributionValues::Column(vec![])));
        assert_eq!(
            reply(ExtensionKind::Decoration, "()").unwrap(),
            Reply::Values(ContributionValues::Decoration(vec![]))
        );
        assert_eq!(reply(ExtensionKind::Sidebar, "()").unwrap(), Reply::Values(ContributionValues::Sidebar(vec![])));
        assert_eq!(reply(ExtensionKind::Status, "()").unwrap(), Reply::Values(ContributionValues::Status(vec![])));
        assert_eq!(reply(ExtensionKind::Info, "()").unwrap(), Reply::Values(ContributionValues::Info(vec![])));
        assert_eq!(
            reply(ExtensionKind::Preview, "()").unwrap(),
            Reply::Values(ContributionValues::Preview(PluginPreview::default()))
        );
        assert_eq!(reply(ExtensionKind::Command, "()").unwrap(), Reply::Unit);
    }

    #[test]
    fn decoration_values_are_typed_and_bounded() {
        let value = reply(
            ExtensionKind::Decoration,
            r#"[ #{ handle: 1, text: "", spoken: "done", tint: "success", dim: true }, #{ text: "ignored" } ]"#,
        )
        .unwrap();
        match value {
            Reply::Values(ContributionValues::Decoration(values)) => {
                assert_eq!(values.len(), 1);
                assert_eq!(values[0].handle, EntryHandle(1));
                assert_eq!(values[0].text, "");
                assert_eq!(values[0].spoken.as_deref(), Some("done"));
                assert_eq!(values[0].tint, Some(Tint::Success));
                assert!(values[0].dim);
            }
            other => panic!("{other:?}"),
        }
        assert!(reply(ExtensionKind::Decoration, r#"[ #{ handle: 1, text: "0123456789abcdefg" } ]"#).is_err());
        assert!(reply(ExtensionKind::Decoration, r#"[ #{ handle: 1, text: "x", tint: "blue" } ]"#).is_err());
        assert!(reply(ExtensionKind::Column, r#"[ #{ handle: 1, text: "x", tint: "accent" } ]"#).is_err());
        assert!(reply(ExtensionKind::Column, r#"[ #{ handle: 1, text: "x", dim: true } ]"#).is_err());
        assert!(reply(ExtensionKind::Decoration, r#"[ #{ handle: 1, text: "x", dim: "yes" } ]"#).is_err());
        assert_eq!(
            reply(ExtensionKind::Decoration, "()").unwrap(),
            Reply::Values(ContributionValues::Decoration(vec![]))
        );
    }

    #[test]
    fn sidebar_rows_accept_a_known_handle_and_bound_the_badge() {
        match reply(ExtensionKind::Sidebar, r#"[ #{ id: "a", title: "A", badge: "B", handle: 1 } ]"#).unwrap() {
            Reply::Values(ContributionValues::Sidebar(rows)) => {
                assert_eq!(rows[0].handle, Some(EntryHandle(1)));
                assert_eq!(rows[0].badge.as_deref(), Some("B"));
            }
            other => panic!("{other:?}"),
        }
        assert!(reply(ExtensionKind::Sidebar, r#"[ #{ id: "a", title: "A", handle: 9 } ]"#).is_err());
        assert!(reply(ExtensionKind::Sidebar, r#"[ #{ id: "a", title: "A", badge: "0123456789abcdefg" } ]"#).is_err());
        assert!(reply(ExtensionKind::Sidebar, r#"[ #{ id: "a", title: "" } ]"#).is_err());
        assert_eq!(reply(ExtensionKind::Sidebar, "()").unwrap(), Reply::Values(ContributionValues::Sidebar(vec![])));
    }

    #[test]
    fn preview_tables_are_rectangular_and_empty_shapes_decline() {
        let default = PluginPreview::default();
        for source in [r#"#{ text: "" }"#, r#"#{ table: [] }"#, r#"#{}"#, r#"#{ table: [[]] }"#] {
            assert_eq!(
                reply(ExtensionKind::Preview, source).unwrap(),
                Reply::Values(ContributionValues::Preview(default.clone())),
                "{source}"
            );
        }
        assert!(reply(ExtensionKind::Preview, r#"#{ table: [["a"], ["b", "c"]] }"#).is_err());
        assert!(reply(ExtensionKind::Preview, r#"#{ text: "x", table: [["a"]] }"#).is_err());
        let wide = format!("#{{ table: [{}] }}", vec!["\"x\""; 65].join(","));
        assert!(reply(ExtensionKind::Preview, &wide).is_err());
        let wide_rows = format!("#{{ table: [{}] }}", vec!["[\"x\"]"; 129].join(","));
        assert!(reply(ExtensionKind::Preview, &wide_rows).is_err());
        match reply(ExtensionKind::Preview, r#"#{ table: [["a", "b"], ["c", "d"]] }"#).unwrap() {
            Reply::Values(ContributionValues::Preview(preview)) => {
                assert_eq!(
                    preview.table.as_deref(),
                    Some(&[vec!["a".into(), "b".into()], vec!["c".into(), "d".into()]][..])
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn command_effects_are_typed_and_handles_are_request_local() {
        assert_eq!(
            reply(ExtensionKind::Command, r#"#{ kind: "message", text: "Done" }"#).unwrap(),
            Reply::Intent(CommandIntent::Message("Done".into()))
        );
        assert_eq!(
            reply(ExtensionKind::Command, r#"#{ kind: "copy_text", text: "a\nb" }"#).unwrap(),
            Reply::Intent(CommandIntent::CopyText("a\nb".into()))
        );
        assert_eq!(
            reply(ExtensionKind::Command, r#"#{ kind: "reveal", handle: 1 }"#).unwrap(),
            Reply::Intent(CommandIntent::Reveal(EntryHandle(1)))
        );
        assert!(reply(ExtensionKind::Command, r#"#{ kind: "message", text: "" }"#).is_err());
        assert!(reply(ExtensionKind::Command, r#"#{ kind: "message", text: "x", handle: 1 }"#).is_err());
        assert!(reply(ExtensionKind::Command, r#"#{ kind: "reveal", handle: 9 }"#).is_err());
        assert!(reply(ExtensionKind::Command, r#"#{ kind: "reveal", handle: 1, text: "x" }"#).is_err());
        assert!(reply(ExtensionKind::Command, r#"#{ kind: "open_url", text: "x" }"#).is_err());
    }

    #[test]
    fn status_and_info_accept_unit_and_empty_display_text() {
        assert_eq!(reply(ExtensionKind::Status, "()").unwrap(), Reply::Values(ContributionValues::Status(vec![])));
        match reply(ExtensionKind::Status, r#"[ #{ text: "" } ]"#).unwrap() {
            Reply::Values(ContributionValues::Status(items)) => assert!(items[0].text.is_empty()),
            other => panic!("{other:?}"),
        }
        assert_eq!(reply(ExtensionKind::Info, "()").unwrap(), Reply::Values(ContributionValues::Info(vec![])));
        assert!(reply(ExtensionKind::Info, r#"[ #{ label: "L", value: "V", extra: 1 } ]"#).is_err());
        assert!(reply(ExtensionKind::Status, r#"[ #{ text: "x", handle: 1 } ]"#).is_err());
    }

    #[test]
    fn the_shipped_p1b_fixture_registers_every_api1_kind() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../crow-app/tests/fixtures/plugins/p1b-fixture");
        let snapshot = crate::package::load_package(&root, &limits()).unwrap();
        let source = std::str::from_utf8(&snapshot.files["main.rhai"]).unwrap();
        let mut engine = rhai::Engine::new();
        engine.set_max_expr_depths(64, 64);
        engine.set_max_array_size(1024);
        engine.set_max_map_size(1024);
        let ast = engine.compile(source).unwrap();
        let value = engine.call_fn::<Dynamic>(&mut rhai::Scope::new(), &ast, "register", ()).unwrap();
        let descriptors =
            validate_registration(&snapshot.manifest.id, &snapshot.manifest, &snapshot, &ast, value, &limits())
                .unwrap();
        assert_eq!(descriptors.len(), 9);
        assert_eq!(descriptors.iter().filter(|d| d.kind == ExtensionKind::Column).count(), 1);
        assert_eq!(descriptors.iter().filter(|d| d.kind == ExtensionKind::Preview).count(), 1);
        let command = descriptors.iter().find(|d| d.kind == ExtensionKind::Command).unwrap();
        assert_eq!(command.applies_to, CommandTarget::Selection);
        assert!(command.context_menu);
        assert_eq!(command.extensions.as_deref(), Some(&["txt".to_owned()][..]));
    }

    #[test]
    fn published_descriptors_are_rechecked_against_the_generation_rules() {
        let snapshot = all_snapshot();
        let mut column = descriptor(ExtensionKind::Column, "c", "C", DescriptorSource::Function("run".into()));
        validate_published(&plugin(), &snapshot, &[column.clone()], &limits()).unwrap();
        column.width = 16;
        assert!(validate_published(&plugin(), &snapshot, &[column.clone()], &limits()).is_err());
        column.width = 100;
        column.context_menu = true;
        assert!(validate_published(&plugin(), &snapshot, &[column.clone()], &limits()).is_err());
        column.context_menu = false;
        column.extensions = Some(vec!["txt".into()]);
        assert!(validate_published(&plugin(), &snapshot, &[column], &limits()).is_err());

        let mut preview = descriptor(ExtensionKind::Preview, "p", "P", DescriptorSource::Function("run".into()));
        preview.extensions = Some(vec!["TXT".into()]);
        assert!(validate_published(&plugin(), &snapshot, &[preview.clone()], &limits()).is_err());
        preview.extensions = Some(vec!["txt".into()]);
        validate_published(&plugin(), &snapshot, &[preview], &limits()).unwrap();
    }
}
