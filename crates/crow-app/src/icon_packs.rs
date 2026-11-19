//! Active icon packs: discovery, bounded off-main loading, one immutable
//! snapshot per generation, and render-time resolution over that snapshot.
//!
//! Local packs live in `<config>/icon-packs/<id>/pack.toml`; plugin packs come
//! from P1B's approved `RegisteredIconPack` snapshot. A reload parses every
//! pack off the main thread; a parse or resource error keeps the entire last
//! good snapshot and reports the error. An outdated reply can never publish.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crow_core::icon_pack::{
    IconColor, IconPack, MAX_GLYPH_BYTES, MAX_GLYPHS, MAX_PACK_ASSET_BYTES, MAX_PACK_BYTES, NATIVE_ICON_SOURCE,
    validate_source, validate_svg,
};
use crow_core::{LocalId, PluginId};
use crow_plugin::RegisteredIconPack;
use gpui::{App, AppContext as _, Global, SharedString};

use crate::ui::Ui;

const MAX_PACKS: usize = 64;

/// One pack after every referenced glyph was read and validated.
struct LoadedPack {
    name: String,
    pack: IconPack,
    assets: HashMap<String, Arc<[u8]>>,
}

/// An immutable set of packs published together. Rows keep an `Arc` of this for
/// a whole render, so one frame never mixes two generations.
#[derive(Clone, Default)]
pub struct IconSnapshot {
    generation: u64,
    packs: BTreeMap<String, Arc<LoadedPack>>,
}

/// One pack the Settings control offers, independent of the saved choice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconPackChoice {
    pub id: String,
    pub name: String,
}

impl IconSnapshot {
    #[cfg(test)]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Every loaded pack by human name, then id, so catalog reloads keep a
    /// stable order. The saved id is added by the caller when it is missing.
    pub fn choices(&self) -> Vec<IconPackChoice> {
        let mut choices: Vec<IconPackChoice> =
            self.packs.iter().map(|(id, pack)| IconPackChoice { id: id.clone(), name: pack.name.clone() }).collect();
        choices.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
        choices
    }

    /// The first matching pack entry for this path, with the exact bytes the
    /// snapshot loaded. `None` means "draw the native icon".
    pub fn resolve(&self, source: &str, path: &Path, is_dir: bool, is_package: bool) -> Option<ResolvedIcon> {
        if source == NATIVE_ICON_SOURCE {
            return None;
        }
        let pack = self.packs.get(source)?;
        let type_ids = if pack.pack.uses_type() {
            let folder_like = is_dir && !is_package;
            if folder_like {
                crow_macos::type_identifiers("", true)
            } else {
                path.extension().and_then(|ext| ext.to_str()).and_then(|ext| crow_macos::type_identifiers(ext, false))
            }
        } else {
            None
        };
        let entry = pack.pack.resolve(path, is_dir, is_package, type_ids.as_deref().unwrap_or(&[]))?;
        let asset = pack.assets.get(&entry.icon)?.clone();
        Some(ResolvedIcon { asset, color: entry.color.clone() })
    }

    /// A display name for a configured source, including the missing case.
    pub fn display_name(&self, source: &str) -> String {
        if source == NATIVE_ICON_SOURCE {
            return "Native".into();
        }
        match self.packs.get(source) {
            Some(pack) => pack.name.clone(),
            None => format!("{source} (not installed)"),
        }
    }
}

/// The bytes and tint a row draws for one path.
#[derive(Clone)]
pub struct ResolvedIcon {
    pub asset: Arc<[u8]>,
    pub color: IconColor,
}

/// Resolve paths for one render against one snapshot and one effective source.
#[derive(Clone, Default)]
pub struct Resolver {
    snapshot: Option<Arc<IconSnapshot>>,
    source: Arc<str>,
}

impl Resolver {
    pub fn for_source(cx: &App, source: &str) -> Self {
        if source == NATIVE_ICON_SOURCE {
            return Self::default();
        }
        let snapshot = cx.try_global::<IconPacks>().map(IconPacks::snapshot);
        Self { snapshot, source: Arc::from(source) }
    }

    pub fn resolve(&self, path: &Path, is_dir: bool, is_package: bool) -> Option<ResolvedIcon> {
        self.snapshot.as_ref()?.resolve(&self.source, path, is_dir, is_package)
    }
}

/// The app-owned pack state. `request` is bumped on every reload; only a reply
/// carrying the newest request may publish.
pub struct IconPacks {
    snapshot: Arc<IconSnapshot>,
    request: u64,
    loading: bool,
    queued: bool,
    error: Option<SharedString>,
    reported_error: Option<String>,
}

impl Global for IconPacks {}

impl Default for IconPacks {
    fn default() -> Self {
        Self {
            snapshot: Arc::new(IconSnapshot::default()),
            request: 0,
            loading: false,
            queued: false,
            error: None,
            reported_error: None,
        }
    }
}

impl IconPacks {
    pub fn snapshot(&self) -> Arc<IconSnapshot> {
        self.snapshot.clone()
    }

    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub fn display_name(&self, source: &str) -> String {
        self.snapshot.display_name(source)
    }
}

/// Set up the global without touching the disk. Startup calls `reload` once the
/// plugin service is running, so plugin packs are in its first snapshot.
pub fn install(cx: &mut App) {
    if cx.try_global::<IconPacks>().is_none() {
        cx.set_global(IconPacks::default());
    }
}

/// Reload every pack in the background. Coalesces overlapping requests and
/// keeps the last good snapshot until a complete, newer load succeeds.
pub fn reload(cx: &mut App) {
    install(cx);
    let config_dir = cx.global::<Ui>().config_dir.clone();
    let home = cx.global::<Ui>().home.clone();
    let plugin_packs: Vec<RegisteredIconPack> = cx
        .try_global::<crate::plugins::Plugins>()
        .map(|plugins| plugins.surfaces.snapshot().packs.values().cloned().collect())
        .unwrap_or_default();
    let generation = {
        let icons = cx.global_mut::<IconPacks>();
        icons.request = icons.request.wrapping_add(1);
        if icons.loading {
            icons.queued = true;
            return;
        }
        icons.loading = true;
        icons.request
    };
    spawn_load(cx, generation, config_dir, home, plugin_packs);
}

fn spawn_load(
    cx: &mut App,
    generation: u64,
    config_dir: PathBuf,
    home: PathBuf,
    plugin_packs: Vec<RegisteredIconPack>,
) {
    let task = cx.background_spawn(async move { load_catalog_with(&config_dir, &home, &plugin_packs) });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| finish_load(cx, generation, result));
    })
    .detach();
}

fn finish_load(cx: &mut App, generation: u64, result: Result<IconSnapshot, String>) {
    let (start, changed) = {
        let icons = cx.global_mut::<IconPacks>();
        let current = generation == icons.request;
        let changed = current && icons.apply(generation, result);
        if icons.queued {
            icons.queued = false;
            icons.loading = true;
            (Some(icons.request), changed)
        } else {
            icons.loading = false;
            (None, changed)
        }
    };
    if let Some(generation) = start {
        let config_dir = cx.global::<Ui>().config_dir.clone();
        let home = cx.global::<Ui>().home.clone();
        let plugin_packs: Vec<RegisteredIconPack> = cx
            .try_global::<crate::plugins::Plugins>()
            .map(|plugins| plugins.surfaces.snapshot().packs.values().cloned().collect())
            .unwrap_or_default();
        spawn_load(cx, generation, config_dir, home, plugin_packs);
    }
    let error = cx.global::<IconPacks>().error.clone();
    if changed {
        cx.refresh_windows();
        // Status errors stay until something replaces them. A successful load
        // that follows a failure is that something: drop the error text we put
        // there, but leave an unrelated message alone.
        if error.is_none() {
            let previous = cx.global_mut::<IconPacks>().reported_error.take();
            let still_shown = previous.is_some_and(|previous| {
                cx.try_global::<crate::operations::Operations>().is_some_and(|operations| {
                    operations.message.as_ref().is_some_and(|(text, is_error)| *is_error && text.as_ref() == previous)
                })
            });
            if still_shown {
                crate::operations::clear_message(cx);
            }
        }
    }
    if let Some(error) = error
        && cx.global::<IconPacks>().reported_error.as_deref() != Some(error.as_ref())
    {
        cx.global_mut::<IconPacks>().reported_error = Some(error.to_string());
        if cx.try_global::<crate::operations::Operations>().is_some() {
            crate::operations::message(error, true, cx);
        }
    }
}

impl IconPacks {
    /// Apply one load reply. Returns whether the published snapshot changed.
    /// A stale generation is ignored; an error keeps the entire last good set.
    fn apply(&mut self, generation: u64, result: Result<IconSnapshot, String>) -> bool {
        if generation != self.request {
            return false;
        }
        match result {
            Ok(mut snapshot) => {
                snapshot.generation = generation;
                self.snapshot = Arc::new(snapshot);
                self.error = None;
                true
            }
            Err(error) => {
                self.error = Some(error.into());
                false
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn begin_for_tests(&mut self) -> u64 {
        self.request = self.request.wrapping_add(1);
        self.request
    }

    #[cfg(test)]
    pub(crate) fn install_for_tests(&mut self, snapshot: IconSnapshot) {
        self.request = self.request.wrapping_add(1);
        self.snapshot = Arc::new(snapshot);
        self.error = None;
    }

    #[cfg(test)]
    pub(crate) fn publish_for_tests(&mut self, generation: u64, result: Result<IconSnapshot, String>) -> bool {
        self.apply(generation, result)
    }
}

pub(crate) trait PluginPackSource {
    fn key(&self) -> String;
    fn bytes(&self) -> Option<Arc<[u8]>>;
    fn asset(&self, name: &str) -> Option<Arc<[u8]>>;
}

impl PluginPackSource for RegisteredIconPack {
    fn key(&self) -> String {
        self.id().to_string()
    }

    fn bytes(&self) -> Option<Arc<[u8]>> {
        self.pack_bytes()
    }

    fn asset(&self, name: &str) -> Option<Arc<[u8]>> {
        self.declared_asset(name)
    }
}

#[cfg(test)]
pub(crate) struct TestPluginPack {
    pub key: String,
    pub pack: Arc<[u8]>,
    pub assets: HashMap<String, Arc<[u8]>>,
}

#[cfg(test)]
impl PluginPackSource for TestPluginPack {
    fn key(&self) -> String {
        self.key.clone()
    }

    fn bytes(&self) -> Option<Arc<[u8]>> {
        Some(self.pack.clone())
    }

    fn asset(&self, name: &str) -> Option<Arc<[u8]>> {
        self.assets.get(name).cloned()
    }
}

#[cfg(test)]
pub(crate) fn load_catalog(config_dir: &Path, home: &Path) -> Result<IconSnapshot, String> {
    load_catalog_with::<RegisteredIconPack>(config_dir, home, &[])
}

pub(crate) fn load_catalog_with<P: PluginPackSource>(
    config_dir: &Path,
    home: &Path,
    plugins: &[P],
) -> Result<IconSnapshot, String> {
    let mut packs = BTreeMap::new();
    for pack in load_local(config_dir, home)? {
        packs.insert(pack.0, pack.1);
    }
    for source in plugins {
        let key = source.key();
        if packs.contains_key(&key) {
            return Err(format!("icon pack \"{key}\": two packs use this id"));
        }
        let bytes = source.bytes().ok_or_else(|| format!("icon pack \"{key}\": its pack.toml is missing"))?;
        let loaded = load_plugin_pack(&key, &bytes, home, source)?;
        packs.insert(key, loaded);
    }
    if packs.len() > MAX_PACKS {
        return Err(format!("more than {MAX_PACKS} icon packs are installed"));
    }
    Ok(IconSnapshot { generation: 0, packs: packs.into_iter().map(|(key, pack)| (key, Arc::new(pack))).collect() })
}

fn load_local(config_dir: &Path, home: &Path) -> Result<Vec<(String, LoadedPack)>, String> {
    let root = config_dir.join("icon-packs");
    // io-ok: the whole catalog loads in background_spawn
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{}: {error}", root.display())),
    };
    let mut packs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", root.display()))?;
        let file_type = entry.file_type().map_err(|error| format!("{}: {error}", entry.path().display()))?;
        let name = entry.file_name();
        let key = name
            .to_str()
            .ok_or_else(|| format!("icon-packs: {} is not valid UTF-8", name.to_string_lossy()))?
            .to_owned();
        if file_type.is_symlink() {
            return Err(format!("icon-packs/{key}: a pack folder may not be a symlink"));
        }
        // io-ok: the whole catalog loads in background_spawn
        if !file_type.is_dir() {
            continue;
        }
        // io-ok: the whole catalog loads in background_spawn
        if !entry.path().join("pack.toml").exists() {
            continue;
        }
        LocalId::parse(&key).map_err(|error| format!("icon-packs/{key}: {error}"))?;
        if key == NATIVE_ICON_SOURCE {
            return Err("icon-packs/native: \"native\" is reserved".into());
        }
        let pack = load_local_pack(&entry.path(), &key, home)?;
        packs.push((key, pack));
    }
    Ok(packs)
}

fn load_local_pack(dir: &Path, key: &str, home: &Path) -> Result<LoadedPack, String> {
    let path = dir.join("pack.toml");
    // io-ok: the whole catalog loads in background_spawn
    let meta = std::fs::symlink_metadata(&path).map_err(|error| format!("{}/pack.toml: {error}", dir.display()))?;
    // io-ok: the whole catalog loads in background_spawn
    if !meta.file_type().is_file() {
        return Err(format!("{}/pack.toml is not a regular file", dir.display()));
    }
    if meta.len() > MAX_PACK_BYTES as u64 {
        return Err(format!("{}/pack.toml is larger than {} KiB", dir.display(), MAX_PACK_BYTES / 1024));
    }
    // io-ok: the whole catalog loads in background_spawn
    let text = std::fs::read_to_string(&path).map_err(|error| format!("{}/pack.toml: {error}", dir.display()))?;
    let pack = IconPack::parse(&text, home).map_err(|error| format!("icon pack \"{key}\": {error}"))?;
    if let Some(id) = pack.id()
        && id != key
    {
        return Err(format!("icon pack \"{key}\": pack.toml says id = \"{id}\""));
    }
    // io-ok: the whole catalog loads in background_spawn
    let root = dir.canonicalize().map_err(|error| format!("{}: {error}", dir.display()))?;
    let mut assets = HashMap::new();
    let mut total = 0usize;
    for reference in pack.assets() {
        let asset = read_local_asset(&root, dir, reference).map_err(|error| format!("icon pack \"{key}\": {error}"))?;
        total = total.saturating_add(asset.len());
        if total > MAX_PACK_ASSET_BYTES {
            return Err(format!(
                "icon pack \"{key}\": glyphs total more than {} MiB",
                MAX_PACK_ASSET_BYTES / 1024 / 1024
            ));
        }
        assets.insert(reference.to_owned(), asset);
    }
    let name = pack.name().map(str::to_owned).unwrap_or_else(|| key.to_owned());
    Ok(LoadedPack { name, pack, assets })
}

fn read_local_asset(root: &Path, dir: &Path, reference: &str) -> Result<Arc<[u8]>, String> {
    let path = dir.join(reference);
    // io-ok: the whole catalog loads in background_spawn
    let meta = std::fs::symlink_metadata(&path).map_err(|error| format!("glyph \"{reference}\": {error}"))?;
    // io-ok: the whole catalog loads in background_spawn
    if !meta.file_type().is_file() {
        return Err(format!("glyph \"{reference}\" is not a regular file"));
    }
    if meta.len() > MAX_GLYPH_BYTES as u64 {
        return Err(format!("glyph \"{reference}\" is larger than {} KiB", MAX_GLYPH_BYTES / 1024));
    }
    // io-ok: the whole catalog loads in background_spawn
    let canonical = path.canonicalize().map_err(|error| format!("glyph \"{reference}\": {error}"))?;
    if !canonical.starts_with(root) {
        return Err(format!("glyph \"{reference}\" escapes the pack folder"));
    }
    // io-ok: the whole catalog loads in background_spawn
    let bytes = std::fs::read(&path).map_err(|error| format!("glyph \"{reference}\": {error}"))?;
    if bytes.len() > MAX_GLYPH_BYTES {
        return Err(format!("glyph \"{reference}\" is larger than {} KiB", MAX_GLYPH_BYTES / 1024));
    }
    validate_svg(&bytes).map_err(|error| format!("glyph \"{reference}\": {error}"))?;
    Ok(Arc::from(bytes))
}

fn load_plugin_pack<P: PluginPackSource>(
    key: &str,
    bytes: &[u8],
    home: &Path,
    source: &P,
) -> Result<LoadedPack, String> {
    validate_source(key).map_err(|error| format!("icon pack \"{key}\": {error}"))?;
    if !key.contains('.') {
        return Err(format!("icon pack \"{key}\": plugin packs use <plugin-id>.<pack-id>"));
    }
    let plugin = key.rsplit_once('.').map(|(plugin, _)| plugin).unwrap_or(key);
    PluginId::parse(plugin).map_err(|error| format!("icon pack \"{key}\": {error}"))?;
    if bytes.len() > MAX_PACK_BYTES {
        return Err(format!("icon pack \"{key}\": pack.toml is larger than {} KiB", MAX_PACK_BYTES / 1024));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| format!("icon pack \"{key}\": pack.toml is not UTF-8"))?;
    let pack = IconPack::parse(text, home).map_err(|error| format!("icon pack \"{key}\": {error}"))?;
    let mut assets = HashMap::new();
    let mut total = 0usize;
    for reference in pack.assets() {
        let asset = source
            .asset(reference)
            .ok_or_else(|| format!("icon pack \"{key}\": glyph \"{reference}\" is not in the approved snapshot"))?;
        if asset.len() > MAX_GLYPH_BYTES {
            return Err(format!(
                "icon pack \"{key}\": glyph \"{reference}\" is larger than {} KiB",
                MAX_GLYPH_BYTES / 1024
            ));
        }
        total = total.saturating_add(asset.len());
        if total > MAX_PACK_ASSET_BYTES {
            return Err(format!(
                "icon pack \"{key}\": glyphs total more than {} MiB",
                MAX_PACK_ASSET_BYTES / 1024 / 1024
            ));
        }
        validate_svg(&asset).map_err(|error| format!("icon pack \"{key}\": glyph \"{reference}\": {error}"))?;
        assets.insert(reference.to_owned(), asset);
    }
    if assets.len() > MAX_GLYPHS {
        return Err(format!("icon pack \"{key}\": more than {MAX_GLYPHS} glyphs"));
    }
    let name = pack.name().map(str::to_owned).unwrap_or_else(|| key.to_owned());
    Ok(LoadedPack { name, pack, assets })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLYPH: &str =
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M4 4h16v16H4z"/></svg>"#;

    fn config(name: &str) -> tempfile::TempDir {
        tempfile::Builder::new().prefix(&format!("crow-icon-packs-{name}-")).tempdir().expect("temp config")
    }

    fn write_pack(dir: &Path, id: &str, toml: &str, glyphs: &[(&str, &str)]) {
        let pack = dir.join("icon-packs").join(id);
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("pack.toml"), toml).unwrap();
        for (name, bytes) in glyphs {
            std::fs::write(pack.join(name), bytes).unwrap();
        }
    }

    #[test]
    fn a_local_pack_loads_its_glyphs_and_resolves_them() {
        let root = config("packs");
        write_pack(
            root.path(),
            "sample",
            r#"
api = 1
name = "Sample"
[file_name]
"Cargo.toml" = { icon = "cargo.svg", color = "accent" }
[extension]
rs = { icon = "rust.svg", color = "base09" }
[type]
"public.image" = { icon = "image.svg", color = "base0E" }
[generic]
file = { icon = "file.svg", color = "fg-muted" }
"#,
            &[("cargo.svg", GLYPH), ("rust.svg", GLYPH), ("file.svg", GLYPH), ("image.svg", GLYPH)],
        );
        let snapshot = load_catalog(root.path(), Path::new("/Users/test")).unwrap();
        assert_eq!(snapshot.display_name("sample"), "Sample");
        let cargo = snapshot.resolve("sample", Path::new("/x/Cargo.toml"), false, false).unwrap();
        assert_eq!(cargo.color, IconColor::Role("accent".into()));
        assert_eq!(cargo.asset.as_ref(), GLYPH.as_bytes());
        let rust = snapshot.resolve("sample", Path::new("/x/lib.rs"), false, false).unwrap();
        assert_eq!(rust.color, IconColor::Slot("base09".into()));
        let image = snapshot.resolve("sample", Path::new("/x/photo.png"), false, false).unwrap();
        assert_eq!(image.color, IconColor::Slot("base0E".into()), "a PNG conforms to public.image");
        assert!(snapshot.resolve("sample", Path::new("/x/app"), true, false).is_none());
    }

    #[test]
    fn a_plugin_pack_loads_from_the_approved_snapshot() {
        let root = config("plugin-packs");
        let toml = Arc::from(b"api = 1\n[extension]\nrs = { icon = \"rust.svg\" }\n".as_slice());
        let mut assets = HashMap::new();
        assets.insert("rust.svg".to_owned(), Arc::from(GLYPH.as_bytes()));
        let plugin = TestPluginPack { key: "example.pack.theme".into(), pack: toml, assets };
        let snapshot = load_catalog_with(root.path(), Path::new("/Users/test"), &[plugin]).unwrap();
        let icon = snapshot.resolve("example.pack.theme", Path::new("/x/a.rs"), false, false).unwrap();
        assert_eq!(icon.asset.as_ref(), GLYPH.as_bytes());
    }

    #[test]
    fn an_undeclared_plugin_glyph_rejects_the_whole_load() {
        let root = config("plugin-undeclared");
        let toml = Arc::from(b"api = 1\n[extension]\nrs = { icon = \"rust.svg\" }\n".as_slice());
        let plugin = TestPluginPack { key: "example.pack.theme".into(), pack: toml, assets: HashMap::new() };
        let error = load_catalog_with(root.path(), Path::new("/Users/test"), &[plugin]).err().unwrap();
        assert!(error.contains("approved snapshot"), "{error}");
    }

    #[test]
    fn escapes_symlinks_active_svg_and_bad_pack_errors_reject_the_load() {
        let root = config("bad-packs");
        write_pack(root.path(), "escape", "api = 1\n[generic]\nfile = { icon = \"../x.svg\" }", &[]);
        assert!(load_catalog(root.path(), Path::new("/Users/test")).is_err());

        let root = config("bad-svg");
        write_pack(
            root.path(),
            "script",
            "api = 1\n[generic]\nfile = { icon = \"x.svg\" }",
            &[("x.svg", "<svg><script/></svg>")],
        );
        let error = load_catalog(root.path(), Path::new("/Users/test")).err().unwrap();
        assert!(error.contains("script"), "{error}");

        let root = config("bad-toml");
        write_pack(root.path(), "toml", "api = 1\nnope = true", &[]);
        assert!(load_catalog(root.path(), Path::new("/Users/test")).is_err());

        let root = config("symlink-asset");
        let dir = root.path().join("icon-packs/sample");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pack.toml"), "api = 1\n[generic]\nfile = { icon = \"x.svg\" }").unwrap();
        let outside = root.path().join("outside.svg");
        std::fs::write(&outside, GLYPH).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("x.svg")).unwrap();
        let error = load_catalog(root.path(), Path::new("/Users/test")).err().unwrap();
        assert!(error.contains("regular file") || error.contains("escapes"), "{error}");
    }

    #[test]
    fn a_stale_generation_cannot_publish_and_an_error_keeps_last_good() {
        let root = config("generations");
        write_pack(
            root.path(),
            "a",
            "api = 1\nname = \"A\"\n[generic]\nfile = { icon = \"a.svg\" }",
            &[("a.svg", GLYPH)],
        );
        write_pack(
            root.path(),
            "b",
            "api = 1\nname = \"B\"\n[generic]\nfile = { icon = \"b.svg\" }",
            &[("b.svg", GLYPH)],
        );
        let good = load_catalog(root.path(), Path::new("/Users/test")).unwrap();
        let mut packs = IconPacks::default();
        let a = packs.begin_for_tests();
        assert!(packs.publish_for_tests(a, Ok(good)));
        let snapshot = packs.snapshot();
        let before = snapshot.generation();

        let a = packs.begin_for_tests();
        let b = packs.begin_for_tests();
        assert!(!packs.publish_for_tests(a, Ok(IconSnapshot::default())), "A's late reply is stale");
        assert_eq!(packs.snapshot().generation(), before, "the last good snapshot stays");
        assert!(packs.publish_for_tests(b, Ok(IconSnapshot::default())), "B publishes");
        assert_eq!(packs.snapshot().generation(), b);

        let error = packs.begin_for_tests();
        assert!(!packs.publish_for_tests(error, Err("broken".into())));
        assert_eq!(&**packs.error().unwrap(), "broken");
        assert_eq!(packs.snapshot().generation(), b, "an error keeps the last good snapshot");
    }

    #[test]
    fn removing_a_local_pack_falls_back_to_native_and_keeps_the_name() {
        let root = config("pack-removal");
        write_pack(
            root.path(),
            "sample",
            "api = 1\nname = \"Sample\"\n[generic]\nfile = { icon = \"file.svg\" }",
            &[("file.svg", GLYPH)],
        );
        let loaded = load_catalog(root.path(), Path::new("/Users/test")).unwrap();
        assert!(loaded.resolve("sample", Path::new("/x/a"), false, false).is_some());
        std::fs::remove_dir_all(root.path().join("icon-packs/sample")).unwrap();
        let removed = load_catalog(root.path(), Path::new("/Users/test")).unwrap();
        assert!(removed.resolve("sample", Path::new("/x/a"), false, false).is_none());
        assert_eq!(removed.display_name("sample"), "sample (not installed)");
    }

    #[test]
    fn a_local_pack_id_must_match_its_folder() {
        let root = config("id-mismatch");
        write_pack(root.path(), "sample", "api = 1\nid = \"other\"\n", &[]);
        let error = load_catalog(root.path(), Path::new("/Users/test")).err().unwrap();
        assert!(error.contains("id = \"other\""), "{error}");
    }

    #[test]
    fn a_pack_id_must_be_a_local_segment_and_native_is_reserved() {
        let root = config("ids");
        for id in ["native", "two.parts", "My-Pack"] {
            write_pack(root.path(), id, "api = 1\n", &[]);
        }
        let error = load_catalog(root.path(), Path::new("/Users/test")).err().unwrap();
        assert!(error.contains("icon-packs"), "{error}");
    }
}
