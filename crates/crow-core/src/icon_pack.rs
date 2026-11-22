//! Data-only icon packs: versioned TOML, bounded SVG validation, and the
//! first-match resolver shared by every view.
//!
//! A pack maps paths, names, extensions, UTType identifiers and the two
//! generic kinds to one of the pack's own SVG files and a theme role or
//! palette hue. Nothing here renders: `crow-app` owns loading, caching and
//! tinting.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crow_theme::{ROLES, Rgb, Theme};
use serde::Deserialize;

use crate::config::expand_home;
use crate::extensions::{LocalId, PluginId};

/// Reserved source: use the native macOS icon.
pub const NATIVE_ICON_SOURCE: &str = "native";

pub const ICON_PACK_API: u32 = 1;
/// A pack TOML is small; this stops a giant file from being parsed.
pub const MAX_PACK_BYTES: usize = 512 * 1024;
/// One glyph is a single-color vector, so it has no reason to be large.
pub const MAX_GLYPH_BYTES: usize = 256 * 1024;
/// Distinct glyph files one pack may reference.
pub const MAX_GLYPHS: usize = 512;
/// Mappings across all tiers of one pack.
pub const MAX_RULES: usize = 4096;
/// All glyph bytes one pack may hold in memory.
pub const MAX_PACK_ASSET_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_ASSET_REF_BYTES: usize = 256;
const MAX_SVG_TAGS: usize = 4096;
const MAX_SVG_DEPTH: usize = 64;

/// Palette hues the spec allows icon packs to name.
const ALLOWED_SLOTS: [&str; 14] = [
    "base08", "base09", "base0A", "base0B", "base0C", "base0D", "base0E", "base0F", "base12", "base13", "base14",
    "base15", "base16", "base17",
];

/// Most packs a source can layer.
pub const MAX_LAYERED_SOURCES: usize = 8;

/// The sources in a (possibly layered) source string: `"lucide, linework"` is two packs,
/// tried in order. A single id is a list of one.
pub fn split_sources(source: &str) -> impl Iterator<Item = &str> {
    source.split(',').map(str::trim).filter(|part| !part.is_empty())
}

/// `native`, a local pack id, or a plugin pack's `<plugin>.<local>` id; or a comma-separated
/// list of them, which Crow tries in order (the first pack with an icon for an item wins, and
/// `native` ends the list early).
pub fn validate_source(source: &str) -> Result<(), String> {
    if source.trim().is_empty() {
        return Err("icon source is empty".into());
    }
    let parts: Vec<&str> = source.split(',').map(str::trim).collect();
    if parts.len() > MAX_LAYERED_SOURCES {
        return Err(format!("icon sources are limited to {MAX_LAYERED_SOURCES} packs"));
    }
    for (i, part) in parts.iter().enumerate() {
        validate_one_source(part)?;
        if parts[..i].contains(part) {
            return Err(format!("icon source \"{part}\" is listed twice"));
        }
    }
    Ok(())
}

fn validate_one_source(source: &str) -> Result<(), String> {
    if source == NATIVE_ICON_SOURCE {
        return Ok(());
    }
    if source.is_empty() {
        return Err("icon source is empty".into());
    }
    if source.len() > PluginId::MAX_BYTES + LocalId::MAX_BYTES + 1 {
        return Err("icon source is too long".into());
    }
    match source.rsplit_once('.') {
        Some((plugin, local)) => {
            PluginId::parse(plugin).map_err(|error| format!("icon source \"{source}\": {error}"))?;
            LocalId::parse(local).map_err(|error| format!("icon source \"{source}\": {error}"))?;
        }
        None => {
            LocalId::parse(source).map_err(|error| format!("icon source \"{source}\": {error}"))?;
        }
    }
    Ok(())
}

/// The source string after the user picks `id` in the pack list: picking a pack that is
/// not on the list adds it after the others, picking one that is removes it, and picking
/// Native clears the list. An empty list is Native.
pub fn toggle_source(current: &str, id: &str) -> String {
    if id == NATIVE_ICON_SOURCE {
        return NATIVE_ICON_SOURCE.to_owned();
    }
    let mut parts: Vec<&str> = split_sources(current).filter(|part| *part != NATIVE_ICON_SOURCE).collect();
    match parts.iter().position(|part| *part == id) {
        Some(at) => {
            parts.remove(at);
        }
        None if parts.len() < MAX_LAYERED_SOURCES => parts.push(id),
        None => {}
    }
    if parts.is_empty() { NATIVE_ICON_SOURCE.to_owned() } else { parts.join(",") }
}

pub fn is_native(source: &str) -> bool {
    split_sources(source).all(|part| part == NATIVE_ICON_SOURCE)
}

/// A theme role or one of the spec's palette hues.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IconColor {
    Role(String),
    Slot(String),
}

impl IconColor {
    pub fn parse(text: &str) -> Result<Self, String> {
        if ROLES.contains(&text) {
            return Ok(Self::Role(text.to_owned()));
        }
        if ALLOWED_SLOTS.contains(&text) {
            return Ok(Self::Slot(text.to_owned()));
        }
        Err(format!(
            "unknown color \"{text}\"; use a theme role ({}) or a palette hue (base08–base0F, base12–base17)",
            ROLES.join(", ")
        ))
    }

    pub fn key(&self) -> &str {
        match self {
            Self::Role(name) | Self::Slot(name) => name,
        }
    }

    /// The color for the active theme. A palette slot an older index lacks
    /// falls back to foreground rather than an arbitrary magenta.
    pub fn rgb(&self, theme: &Theme) -> Rgb {
        match self {
            Self::Role(name) => theme.role(name),
            Self::Slot(name) => theme.slot(name).unwrap_or_else(|| theme.role("fg")),
        }
    }
}

/// One mapping target: a bundle-relative SVG plus its tint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconEntry {
    pub icon: String,
    pub color: IconColor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    icon: String,
    #[serde(default)]
    color: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawGeneric {
    #[serde(default)]
    file: Option<RawEntry>,
    #[serde(default)]
    folder: Option<RawEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPack {
    api: u32,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    item: BTreeMap<String, RawEntry>,
    #[serde(default, rename = "file_name")]
    file_name: BTreeMap<String, RawEntry>,
    #[serde(default, rename = "folder_name")]
    folder_name: BTreeMap<String, RawEntry>,
    #[serde(default)]
    extension: BTreeMap<String, RawEntry>,
    #[serde(default, rename = "type")]
    type_id: BTreeMap<String, RawEntry>,
    #[serde(default)]
    generic: RawGeneric,
}

/// A validated pack. All referenced glyphs are declared here; the app checks
/// that each exists and is a safe, bounded SVG.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconPack {
    id: Option<String>,
    name: Option<String>,
    item: BTreeMap<PathBuf, IconEntry>,
    file_name: BTreeMap<String, IconEntry>,
    folder_name: BTreeMap<String, IconEntry>,
    extension: BTreeMap<String, IconEntry>,
    type_id: BTreeMap<String, IconEntry>,
    generic_file: Option<IconEntry>,
    generic_folder: Option<IconEntry>,
}

impl IconPack {
    pub fn parse(text: &str, home: &Path) -> Result<Self, String> {
        if text.len() > MAX_PACK_BYTES {
            return Err(format!("pack.toml is larger than {} KiB", MAX_PACK_BYTES / 1024));
        }
        let raw: RawPack = toml::from_str(text).map_err(|error| error.to_string())?;
        if raw.api != ICON_PACK_API {
            return Err(format!("pack api {} is not supported (expected {ICON_PACK_API})", raw.api));
        }
        if let Some(id) = &raw.id {
            LocalId::parse(id).map_err(|error| format!("pack id: {error}"))?;
        }
        let name = match raw.name {
            Some(name) => {
                let name = name.trim();
                if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
                    return Err("pack name must be 1–128 printable characters".into());
                }
                Some(name.to_owned())
            }
            None => None,
        };

        let mut total = 0usize;
        let mut items = BTreeMap::new();
        for (key, raw_entry) in raw.item {
            let path = expand_home(&key, home);
            if !path.is_absolute() {
                return Err(format!("item \"{key}\": use an absolute path or one starting with ~"));
            }
            total += 1;
            if total > MAX_RULES {
                return Err(format!("pack has more than {MAX_RULES} mappings"));
            }
            let entry = convert(raw_entry, &format!("item \"{key}\""))?;
            if items.insert(path, entry).is_some() {
                return Err(format!("item \"{key}\": duplicate path after expanding ~"));
            }
        }
        let file_name = convert_table(raw.file_name, "file_name", false, &mut total)?;
        let folder_name = convert_table(raw.folder_name, "folder_name", false, &mut total)?;
        let extension = convert_table(raw.extension, "extension", true, &mut total)?;
        let type_id = convert_table(raw.type_id, "type", true, &mut total)?;
        if total > MAX_RULES {
            return Err(format!("pack has more than {MAX_RULES} mappings"));
        }
        let generic_file = raw.generic.file.map(|entry| convert(entry, "generic.file")).transpose()?;
        let generic_folder = raw.generic.folder.map(|entry| convert(entry, "generic.folder")).transpose()?;

        let mut assets = BTreeSet::new();
        for entry in items
            .values()
            .chain(file_name.values())
            .chain(folder_name.values())
            .chain(extension.values())
            .chain(type_id.values())
            .chain(generic_file.iter())
            .chain(generic_folder.iter())
        {
            assets.insert(entry.icon.as_str());
        }
        if assets.len() > MAX_GLYPHS {
            return Err(format!("pack references more than {MAX_GLYPHS} glyphs"));
        }

        Ok(Self {
            id: raw.id,
            name,
            item: items,
            file_name,
            folder_name,
            extension,
            type_id,
            generic_file,
            generic_folder,
        })
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn uses_type(&self) -> bool {
        !self.type_id.is_empty()
    }

    /// Every glyph file this pack references, once, in tier order.
    pub fn assets(&self) -> impl Iterator<Item = &str> {
        let mut seen = BTreeSet::new();
        self.entries().map(|entry| entry.icon.as_str()).filter(move |icon| seen.insert(*icon))
    }

    fn entries(&self) -> impl Iterator<Item = &IconEntry> {
        self.item
            .values()
            .chain(self.file_name.values())
            .chain(self.folder_name.values())
            .chain(self.extension.values())
            .chain(self.type_id.values())
            .chain(self.generic_file.iter())
            .chain(self.generic_folder.iter())
    }

    /// First match wins: exact item, exact name (file or folder), extension,
    /// UTType (exact then its supertypes), then generic. Names are matched on
    /// the raw path bytes when they are UTF-8; a lossy display name is never used.
    pub fn resolve(&self, path: &Path, is_dir: bool, is_package: bool, type_ids: &[Arc<str>]) -> Option<&IconEntry> {
        if let Some(entry) = self.item.get(path) {
            return Some(entry);
        }
        let folder_like = is_dir && !is_package;
        let name = path.file_name().and_then(|name| name.to_str());
        if folder_like {
            if let Some(entry) = name.and_then(|name| self.folder_name.get(name)) {
                return Some(entry);
            }
        } else if let Some(entry) = name.and_then(|name| self.file_name.get(name)) {
            return Some(entry);
        }
        if !folder_like
            && let Some(ext) = path.extension().and_then(|ext| ext.to_str())
            && let Some(entry) = self.extension.get(&ext.to_ascii_lowercase())
        {
            return Some(entry);
        }
        for type_id in type_ids {
            if let Some(entry) = self.type_id.get(&type_id.to_ascii_lowercase()) {
                return Some(entry);
            }
        }
        if folder_like { self.generic_folder.as_ref() } else { self.generic_file.as_ref() }
    }
}

fn convert_table(
    raw: BTreeMap<String, RawEntry>,
    what: &str,
    normalize: bool,
    total: &mut usize,
) -> Result<BTreeMap<String, IconEntry>, String> {
    let mut out = BTreeMap::new();
    for (key, raw_entry) in raw {
        *total += 1;
        if *total > MAX_RULES {
            return Err(format!("pack has more than {MAX_RULES} mappings"));
        }
        let key = validate_key(what, &key, normalize)?;
        let entry = convert(raw_entry, &format!("{what} \"{key}\""))?;
        if out.insert(key.clone(), entry).is_some() {
            return Err(format!("{what} \"{key}\": duplicate after case normalization"));
        }
    }
    Ok(out)
}

fn validate_key(what: &str, key: &str, normalize: bool) -> Result<String, String> {
    let key = if what == "extension" { key.strip_prefix('.').unwrap_or(key) } else { key };
    if key.is_empty() || key.len() > 255 || key.chars().any(|ch| ch.is_control() || ch == '/') {
        return Err(format!("{what} \"{key}\": use 1–255 printable characters without /"));
    }
    if what == "type"
        && (key.contains(char::is_whitespace) || !key.is_ascii() || key.starts_with('.') || key.ends_with('.'))
    {
        return Err(format!("type \"{key}\": use an ASCII UTType identifier such as public.image"));
    }
    Ok(if normalize { key.to_ascii_lowercase() } else { key.to_owned() })
}

fn convert(raw: RawEntry, where_: &str) -> Result<IconEntry, String> {
    validate_asset_ref(&raw.icon).map_err(|error| format!("{where_}: {error}"))?;
    let color = match raw.color {
        Some(color) => IconColor::parse(&color).map_err(|error| format!("{where_}: {error}"))?,
        None => IconColor::Role("fg".into()),
    };
    Ok(IconEntry { icon: raw.icon, color })
}

/// A glyph must be one safe, bundle-relative `.svg` path.
pub fn validate_asset_ref(reference: &str) -> Result<(), String> {
    if reference.is_empty() || reference.len() > MAX_ASSET_REF_BYTES {
        return Err(format!("glyph path must be 1–{MAX_ASSET_REF_BYTES} bytes"));
    }
    if reference.starts_with('/') || reference.starts_with('~') {
        return Err(format!("glyph \"{reference}\" must be relative to the pack"));
    }
    if reference.contains(['\\', ':', '\0']) || reference.chars().any(char::is_control) {
        return Err(format!("glyph \"{reference}\" must be a plain relative path"));
    }
    if !reference.to_ascii_lowercase().ends_with(".svg") {
        return Err(format!("glyph \"{reference}\" must be an .svg file"));
    }
    if reference.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return Err(format!("glyph \"{reference}\" must not escape the pack with . or .."));
    }
    Ok(())
}

/// Validate a single-color vector glyph before it reaches GPUI: bounded,
/// UTF-8, well-formed enough to reject broken markup, and free of scripts,
/// external references, embedded images and active attributes.
pub fn validate_svg(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("glyph is empty".into());
    }
    if bytes.len() > MAX_GLYPH_BYTES {
        return Err(format!("glyph is larger than {} KiB", MAX_GLYPH_BYTES / 1024));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "glyph is not UTF-8".to_owned())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut scanner = SvgScanner { text, at: 0, stack: Vec::new(), roots: 0, root_closed: false, tags: 0 };
    scanner.run()?;
    if !scanner.stack.is_empty() {
        return Err(format!("glyph leaves <{}> unclosed", scanner.stack.last().expect("checked")));
    }
    if scanner.roots == 0 {
        return Err("glyph has no <svg> root".into());
    }
    if scanner.roots > 1 {
        return Err("glyph has more than one root element".into());
    }
    if !scanner.root_closed {
        return Err("glyph's <svg> root is not closed".into());
    }
    Ok(())
}

struct SvgScanner<'a> {
    text: &'a str,
    at: usize,
    stack: Vec<String>,
    roots: usize,
    root_closed: bool,
    tags: usize,
}

impl SvgScanner<'_> {
    fn run(&mut self) -> Result<(), String> {
        let bytes = self.text.as_bytes();
        while self.at < bytes.len() {
            match bytes[self.at] {
                b'<' => self.tag()?,
                b'&' => self.entity()?,
                byte if self.stack.is_empty() && !byte.is_ascii_whitespace() => {
                    return Err(if self.roots == 0 {
                        "glyph has text before its root element".into()
                    } else {
                        "glyph has text outside its root element".into()
                    });
                }
                _ => self.at += 1,
            }
        }
        Ok(())
    }

    fn tag(&mut self) -> Result<(), String> {
        let rest = &self.text[self.at..];
        if rest.starts_with("<!--") {
            return self.skip_until("-->", "comment");
        }
        if rest.starts_with("<?") {
            return self.skip_until("?>", "processing instruction");
        }
        if rest.starts_with("<![CDATA[") {
            return self.skip_until("]]>", "CDATA section");
        }
        if rest.starts_with("<!") {
            return Err("glyph may not declare a DOCTYPE or entity".into());
        }
        if rest.starts_with("</") {
            return self.close_tag();
        }
        self.open_tag()
    }

    fn close_tag(&mut self) -> Result<(), String> {
        self.at += 2;
        let name = self.name()?;
        self.skip_space();
        self.expect(b'>')?;
        let Some(open) = self.stack.pop() else {
            return Err(format!("glyph closes </{name}> without opening it"));
        };
        if open != name {
            return Err(format!("glyph closes </{name}> while <{open}> is open"));
        }
        if name == "svg" {
            self.root_closed = true;
        }
        Ok(())
    }

    fn open_tag(&mut self) -> Result<(), String> {
        self.at += 1;
        let name = self.name()?;
        self.tags += 1;
        if self.tags > MAX_SVG_TAGS {
            return Err("glyph has too many elements".into());
        }
        if self.stack.len() >= MAX_SVG_DEPTH {
            return Err("glyph is nested too deeply".into());
        }
        let lower = name.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "script"
                | "style"
                | "link"
                | "base"
                | "foreignobject"
                | "image"
                | "iframe"
                | "audio"
                | "video"
                | "canvas"
                | "object"
                | "embed"
        ) {
            return Err(format!("glyph may not contain <{name}>"));
        }
        let mut self_closing = false;
        loop {
            self.skip_space();
            match self.text.as_bytes().get(self.at) {
                Some(b'>') => {
                    self.at += 1;
                    break;
                }
                Some(b'/') if self.text.as_bytes().get(self.at + 1) == Some(&b'>') => {
                    self.at += 2;
                    self_closing = true;
                    break;
                }
                None => return Err(format!("glyph's <{name}> is not closed")),
                _ => self.attribute(&name)?,
            }
        }
        if self.stack.is_empty() {
            self.roots += 1;
            if self.roots > 1 {
                return Err("glyph has more than one root element".into());
            }
            if name != "svg" {
                return Err(format!("glyph's root is <{name}>, not <svg>"));
            }
        }
        if self_closing {
            if name == "svg" {
                self.root_closed = true;
            }
        } else {
            self.stack.push(name);
        }
        Ok(())
    }

    fn attribute(&mut self, element: &str) -> Result<(), String> {
        let name = self.name()?;
        self.skip_space();
        self.expect(b'=')?;
        self.skip_space();
        let quote = match self.text.as_bytes().get(self.at) {
            Some(b'"') => b'"',
            Some(b'\'') => b'\'',
            _ => return Err(format!("glyph's <{element}> attribute \"{name}\" has no quoted value")),
        };
        self.at += 1;
        let start = self.at;
        while let Some(byte) = self.text.as_bytes().get(self.at) {
            if *byte == quote {
                break;
            }
            self.at += 1;
        }
        if self.at >= self.text.len() {
            return Err(format!("glyph's <{element}> attribute \"{name}\" is not closed"));
        }
        let value = &self.text[start..self.at];
        self.at += 1;
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("on") {
            return Err(format!("glyph's <{element}> may not use the {name} event attribute"));
        }
        if lower == "href" || lower == "xlink:href" || lower == "src" {
            if !value.trim_start().starts_with('#') {
                return Err(format!("glyph's <{element}> may not reference {name}=\"{value}\""));
            }
        } else if lower != "xmlns" && !lower.starts_with("xmlns:") {
            check_css_references(value).map_err(|error| format!("glyph's <{element}> {name}: {error}"))?;
        }
        Ok(())
    }

    fn entity(&mut self) -> Result<(), String> {
        let rest = &self.text[self.at..];
        let Some(end) = rest.find(';') else {
            return Err("glyph has a bare & outside text".into());
        };
        if end > 16 {
            return Err("glyph has an invalid entity reference".into());
        }
        self.at += end + 1;
        Ok(())
    }

    fn skip_until(&mut self, needle: &str, what: &str) -> Result<(), String> {
        let rest = &self.text[self.at..];
        let Some(end) = rest.find(needle) else {
            return Err(format!("glyph's {what} is not closed"));
        };
        self.at += end + needle.len();
        Ok(())
    }

    fn name(&mut self) -> Result<String, String> {
        let bytes = self.text.as_bytes();
        let start = self.at;
        match bytes.get(start) {
            Some(byte) if byte.is_ascii_alphabetic() || matches!(byte, b'_' | b':') => {}
            _ => return Err("glyph has an invalid element or attribute name".into()),
        }
        self.at += 1;
        while let Some(byte) = bytes.get(self.at) {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'.' | b'-') {
                self.at += 1;
            } else {
                break;
            }
        }
        Ok(self.text[start..self.at].to_owned())
    }

    fn skip_space(&mut self) {
        while self.text.as_bytes().get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.text.as_bytes().get(self.at) == Some(&byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!("glyph is missing '{}'", byte as char))
        }
    }
}

fn check_css_references(value: &str) -> Result<(), String> {
    let lower = value.to_ascii_lowercase();
    if ["javascript:", "data:", "file:", "http:", "https:", "@import", "expression("]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        return Err("external or active reference".into());
    }
    let mut rest = lower.as_str();
    while let Some(at) = rest.find("url(") {
        let after = rest[at + 4..].trim_start();
        if !after.starts_with('#') {
            return Err("external url() reference".into());
        }
        rest = after;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layered_sources_validate_split_and_toggle() {
        assert!(validate_source("lucide, linework").is_ok());
        assert!(validate_source("lucide,native").is_ok());
        assert!(validate_source("lucide,lucide").is_err(), "a pack twice");
        assert!(validate_source("lucide,").is_err());
        assert!(validate_source("a,b,c,d,e,f,g,h,i").is_err(), "more than eight");
        assert_eq!(split_sources("a, b ,c").collect::<Vec<_>>(), ["a", "b", "c"]);
        assert!(is_native("native") && !is_native("native,lucide"));
        assert_eq!(toggle_source("native", "lucide"), "lucide", "the first pick replaces Native");
        assert_eq!(toggle_source("lucide", "linework"), "lucide,linework", "later picks are added after");
        assert_eq!(toggle_source("lucide,linework", "lucide"), "linework", "picking a member removes it");
        assert_eq!(toggle_source("lucide", "lucide"), "native", "an empty list is Native");
        assert_eq!(toggle_source("lucide,linework", "native"), "native");
    }

    /// The pack shipped in docs/examples/icon-pack must keep loading: the parser accepts
    /// pack.toml and every glyph it names is a valid SVG inside the pack's icons/ folder.
    #[test]
    fn the_example_icon_pack_in_the_docs_is_valid() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples/icon-pack");
        let text = std::fs::read_to_string(dir.join("pack.toml")).expect("pack.toml");
        let pack = IconPack::parse(&text, std::path::Path::new("/home/example")).expect("the example parses");
        assert_eq!(pack.id(), Some("lucide"));
        let assets: Vec<&str> = pack.assets().collect();
        assert!(assets.len() >= 60, "the example maps many icons");
        for asset in assets {
            assert!(asset.starts_with("icons/"), "{asset} lives in the icons subfolder");
            let bytes = std::fs::read(dir.join(asset)).unwrap_or_else(|e| panic!("{asset}: {e}"));
            validate_svg(&bytes).unwrap_or_else(|e| panic!("{asset}: {e}"));
        }
    }
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    fn pack(text: &str) -> IconPack {
        IconPack::parse(text, Path::new("/Users/test")).expect("pack parses")
    }

    #[test]
    fn parses_the_documented_sample_and_resolves_each_tier() {
        let pack = pack(
            r##"
api = 1
id = "sample"
name = "Sample"

[item]
"/Users/test/work/exact.txt" = { icon = "exact.svg", color = "accent" }

[file_name]
"Cargo.toml" = { icon = "cargo.svg", color = "accent" }

[folder_name]
"src" = { icon = "src.svg", color = "base0D" }

[extension]
rs = { icon = "rust.svg", color = "base09" }

[type]
"public.image" = { icon = "image.svg", color = "base0E" }

[generic]
file = { icon = "file.svg", color = "fg-muted" }
folder = { icon = "folder.svg", color = "base0D" }
"##,
        );
        let exact = Path::new("/Users/test/work/exact.txt");
        assert_eq!(pack.resolve(exact, false, false, &[]).unwrap().icon, "exact.svg");
        assert_eq!(
            pack.resolve(Path::new("/x/Cargo.toml"), false, false, &[]).unwrap().icon,
            "cargo.svg",
            "an exact file name beats the extension tier"
        );
        assert_eq!(pack.resolve(Path::new("/x/src"), true, false, &[]).unwrap().icon, "src.svg");
        assert_eq!(pack.resolve(Path::new("/x/lib.RS"), false, false, &[]).unwrap().icon, "rust.svg");
        assert_eq!(
            pack.resolve(Path::new("/x/photo.raw"), false, false, &[Arc::from("public.image")]).unwrap().icon,
            "image.svg"
        );
        assert_eq!(pack.resolve(Path::new("/x/unknown.bin"), false, false, &[]).unwrap().icon, "file.svg");
        assert_eq!(pack.resolve(Path::new("/x/unknown"), true, false, &[]).unwrap().icon, "folder.svg");
    }

    #[test]
    fn a_missing_tier_tries_the_next_one() {
        let pack = pack(
            r##"
api = 1
[folder_name]
"src" = { icon = "src.svg" }
[generic]
folder = { icon = "folder.svg" }
"##,
        );
        assert_eq!(pack.resolve(Path::new("/x/other"), true, false, &[]).unwrap().icon, "folder.svg");
        assert!(pack.resolve(Path::new("/x/other.txt"), false, false, &[]).is_none());
    }

    #[test]
    fn names_and_folders_match_exactly_but_extensions_and_types_fold_ascii_case() {
        let pack = pack(
            r##"
api = 1
[file_name]
"Makefile" = { icon = "make.svg" }
[folder_name]
"Sources" = { icon = "sources.svg" }
[extension]
"RS" = { icon = "rust.svg" }
[type]
"PUBLIC.IMAGE" = { icon = "image.svg" }
"##,
        );
        assert!(pack.resolve(Path::new("/x/makefile"), false, false, &[]).is_none());
        assert!(pack.resolve(Path::new("/x/sources"), true, false, &[]).is_none());
        assert_eq!(pack.resolve(Path::new("/x/lib.rs"), false, false, &[]).unwrap().icon, "rust.svg");
        assert_eq!(
            pack.resolve(Path::new("/x/a"), false, false, &[Arc::from("public.image")]).unwrap().icon,
            "image.svg"
        );
    }

    #[test]
    fn the_exact_type_beats_a_supertype_and_supertypes_are_tried_in_order() {
        let pack = pack(
            r##"
api = 1
[type]
"public.png" = { icon = "png.svg" }
"public.image" = { icon = "image.svg" }
"public.data" = { icon = "data.svg" }
"##,
        );
        let exact: Arc<[Arc<str>]> =
            Arc::from([Arc::from("public.png"), Arc::from("public.image"), Arc::from("public.data")]);
        assert_eq!(pack.resolve(Path::new("/x/a"), false, false, &exact).unwrap().icon, "png.svg");
        let image: Arc<[Arc<str>]> = Arc::from([Arc::from("public.image"), Arc::from("public.data")]);
        assert_eq!(pack.resolve(Path::new("/x/a"), false, false, &image).unwrap().icon, "image.svg");
    }

    #[test]
    fn non_utf8_names_never_match_a_lossy_name_but_their_extension_still_can() {
        let pack = pack(
            r##"
api = 1
[file_name]
"café.txt" = { icon = "cafe.svg" }
[extension]
txt = { icon = "text.svg" }
[generic]
file = { icon = "file.svg" }
"##,
        );
        let raw = PathBuf::from(OsStr::from_bytes(b"/x/caf\xe9.txt"));
        assert_eq!(pack.resolve(&raw, false, false, &[]).unwrap().icon, "text.svg");
        assert_eq!(pack.resolve(Path::new("/x/café.txt"), false, false, &[]).unwrap().icon, "cafe.svg");
    }

    #[test]
    fn packages_use_file_tiers_not_folder_tiers() {
        let pack = pack(
            r##"
api = 1
[folder_name]
"App.app" = { icon = "folder.svg" }
[extension]
app = { icon = "app.svg" }
[generic]
file = { icon = "file.svg" }
folder = { icon = "folder.svg" }
"##,
        );
        assert_eq!(pack.resolve(Path::new("/x/App.app"), true, true, &[]).unwrap().icon, "app.svg");
    }

    #[test]
    fn item_paths_expand_home_and_must_be_absolute() {
        let pack = pack(
            r##"
api = 1
[item]
"~/Pictures" = { icon = "pictures.svg" }
"##,
        );
        assert_eq!(pack.resolve(Path::new("/Users/test/Pictures"), true, false, &[]).unwrap().icon, "pictures.svg");
        let error = IconPack::parse("api = 1\n[item]\n\"relative\" = { icon = \"x.svg\" }", Path::new("/Users/test"))
            .unwrap_err();
        assert!(error.contains("absolute"), "{error}");
    }

    #[test]
    fn unknown_roles_and_bad_glyph_paths_reject_the_pack() {
        for text in [
            "api = 1\n[generic]\nfile = { icon = \"x.svg\", color = \"nope\" }",
            "api = 1\n[generic]\nfile = { icon = \"../x.svg\" }",
            "api = 1\n[generic]\nfile = { icon = \"/tmp/x.svg\" }",
            "api = 1\n[generic]\nfile = { icon = \"x.png\" }",
            "api = 1\n[generic]\nfile = { icon = \"a//b.svg\" }",
            "api = 2\n",
        ] {
            assert!(IconPack::parse(text, Path::new("/home")).is_err(), "{text:?}");
        }
    }

    #[test]
    fn duplicate_extensions_after_normalization_reject_the_pack() {
        let error = IconPack::parse(
            "api = 1\n[extension]\nrs = { icon = \"a.svg\" }\nRS = { icon = \"b.svg\" }",
            Path::new("/home"),
        )
        .unwrap_err();
        assert!(error.contains("duplicate"), "{error}");
    }

    #[test]
    fn source_ids_reserve_native_and_accept_local_and_plugin_packs() {
        assert!(validate_source("native").is_ok());
        assert!(validate_source("sample").is_ok());
        assert!(validate_source("example.pack.theme").is_ok());
        assert!(validate_source("").is_err());
        assert!(validate_source("Native").is_err());
        assert!(validate_source("my pack").is_err());
        assert!(validate_source("example..theme").is_err());
    }

    #[test]
    fn colors_resolve_through_the_theme() {
        let theme = crow_theme::fallback();
        let color = IconColor::parse("accent").unwrap();
        assert_eq!(color.rgb(&theme), theme.role("accent"));
        let slot = IconColor::parse("base0D").unwrap();
        assert_eq!(slot.rgb(&theme), theme.slot("base0D").unwrap());
        assert!(IconColor::parse("base10").is_err(), "background slots are not icon hues");
    }

    const VALID: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M4 4h16v16H4z" fill="none" stroke="currentColor" stroke-width="2"/></svg>"#;

    #[test]
    fn accepts_a_plain_lucide_style_glyph() {
        validate_svg(VALID.as_bytes()).expect("valid");
        validate_svg(format!("\u{feff}  {VALID}\n").as_bytes()).expect("BOM and padding");
        validate_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="g"/></defs><use href="#g"/></svg>"##
                .as_bytes(),
        )
        .expect("internal use");
    }

    #[test]
    fn rejects_active_and_external_svg_content() {
        for text in [
            r#"<svg><script>alert(1)</script></svg>"#,
            r#"<svg onload="alert(1)"></svg>"#,
            r#"<svg><image href="x.png"/></svg>"#,
            r#"<svg><use href="other.svg#a"/></svg>"#,
            r#"<svg><use href="https://example.com/a.svg#a"/></svg>"#,
            r#"<svg><foreignObject><div/></foreignObject></svg>"#,
            r#"<svg><style>@import url(https://x);</style></svg>"#,
            r#"<svg><path style="fill:url(https://x)"/></svg>"#,
            "<!DOCTYPE svg><svg/>",
            r#"<svg><path d="M0 0"></svg>"#,
            r#"<svg><path d="M0 0"/>"#,
            r#"<svg><g></svg>"#,
            r#"<div/>"#,
        ] {
            assert!(validate_svg(text.as_bytes()).is_err(), "accepted {text:?}");
        }
        assert!(validate_svg(b"not svg").is_err());
        assert!(validate_svg(&[0xff, 0xfe]).is_err());
        let huge = vec![b'x'; MAX_GLYPH_BYTES + 1];
        assert!(validate_svg(&huge).is_err());
    }
}
