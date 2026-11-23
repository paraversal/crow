//! Built-in themes plus a reader for the optional global theme index (`~/.config/themes`).
//!
//! Implements the reader protocol: Base24 palettes plus a fixed set of semantic
//! roles. The built-in themes are compiled in; the index is read when it exists
//! and wins on a shared id. Crow never writes theme files: only "apply to all
//! apps" writes the selection into the state directory, atomically.

use std::collections::{BTreeSet, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Deserialize;

/// Every role in the contract, in a fixed order.
pub const ROLES: [&str; 16] = [
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

/// The themes compiled into the binary: `(id, scheme JSON)`.
const BUILTIN: [(&str, &str); 10] = [
    ("finder-light", include_str!("../themes/finder-light.json")),
    ("finder-dark", include_str!("../themes/finder-dark.json")),
    ("catppuccin-latte", include_str!("../themes/catppuccin-latte.json")),
    ("catppuccin-mocha", include_str!("../themes/catppuccin-mocha.json")),
    ("gruvbox-light", include_str!("../themes/gruvbox-light.json")),
    ("gruvbox-dark", include_str!("../themes/gruvbox-dark.json")),
    ("solarized-light", include_str!("../themes/solarized-light.json")),
    ("solarized-dark", include_str!("../themes/solarized-dark.json")),
    ("rose-pine-dawn", include_str!("../themes/rose-pine-dawn.json")),
    ("rose-pine", include_str!("../themes/rose-pine.json")),
];
/// The base role mapping, used when the index has no (valid) `roles.json`.
const BASE_ROLES: &str = r##"{
  "bg": "base00", "bg-inset": "base10", "surface": "base01", "surface-raised": "base02",
  "border": "base02", "selection": "base02", "fg": "base05", "fg-muted": "base04",
  "fg-subtle": "base03", "accent": "base0D", "on-accent": "base00", "link": "base0D",
  "info": "base0C", "success": "base0B", "warning": "base0A", "error": "base08"
}"##;

/// A 24-bit color, `0xRRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u32);

impl Rgb {
    pub fn parse(s: &str) -> Option<Rgb> {
        let hex = s.strip_prefix('#')?;
        if hex.len() != 6 {
            return None;
        }
        u32::from_str_radix(hex, 16).ok().map(Rgb)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Dark,
    Light,
}

impl Variant {
    pub fn key(self) -> &'static str {
        match self {
            Variant::Dark => "dark",
            Variant::Light => "light",
        }
    }
}

/// A fully resolved theme: every role has a color.
#[derive(Clone, Debug)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub variant: Variant,
    pub palette: HashMap<String, Rgb>,
    roles: HashMap<String, Rgb>,
    /// Roles that follow the system (`"system:accent"`, `"system:selection"`), by role.
    /// Until `resolve_system` runs they hold their palette stand-in color.
    system: HashMap<String, String>,
}

impl Theme {
    /// Roles that follow a system color, with the reference (`"system:accent"`).
    pub fn system_refs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.system.iter().map(|(role, reference)| (role.as_str(), reference.as_str()))
    }

    /// Apply the system colors: `resolver` maps a reference such as `"system:accent"` to a
    /// color. References it doesn't know keep their stand-in.
    pub fn resolve_system(mut self, resolver: &dyn Fn(&str) -> Option<Rgb>) -> Theme {
        for (role, reference) in self.system.clone() {
            if let Some(color) = resolver(&reference) {
                self.roles.insert(role, color);
            }
        }
        self
    }

    /// The color for a role. Roles missing from an older index fall back to the
    /// embedded theme, so this never fails.
    pub fn role(&self, role: &str) -> Rgb {
        self.roles.get(role).or_else(|| cached_fallback(self.variant).roles.get(role)).copied().unwrap_or(Rgb(0xff00ff))
    }

    /// A palette slot, for the many-hue cases (icons, syntax) the protocol allows.
    pub fn slot(&self, slot: &str) -> Option<Rgb> {
        self.palette.get(slot).copied()
    }

    /// The colors `t` of the way from `self` to `to` (0 = self, 1 = to), mixed in
    /// OKLab so a fade between themes stays even in lightness. Names and ids are `to`'s.
    pub fn mix(&self, to: &Theme, t: f32) -> Theme {
        let t = t.clamp(0.0, 1.0);
        let blend = |from: &HashMap<String, Rgb>, target: &HashMap<String, Rgb>| {
            target.iter().map(|(k, &b)| (k.clone(), from.get(k).map_or(b, |&a| a.mix(b, t)))).collect::<HashMap<_, _>>()
        };
        let mut roles = blend(&self.roles, &to.roles);
        for role in ROLES {
            if !roles.contains_key(role) {
                roles.insert(role.to_owned(), self.role(role).mix(to.role(role), t));
            }
        }
        Theme {
            id: to.id.clone(),
            name: to.name.clone(),
            variant: to.variant,
            palette: blend(&self.palette, &to.palette),
            roles,
            system: to.system.clone(),
        }
    }

    /// Same colors for every role and slot.
    pub fn same_colors(&self, other: &Theme) -> bool {
        self.palette == other.palette && ROLES.iter().all(|r| self.role(r) == other.role(r))
    }
}

impl Rgb {
    /// `t` of the way to `to`, in OKLab.
    pub fn mix(self, to: Rgb, t: f32) -> Rgb {
        if self == to {
            return self;
        }
        let (a, b) = (oklab(self), oklab(to));
        let lerp = |x: f32, y: f32| x + (y - x) * t;
        from_oklab([lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2])])
    }
}

fn to_linear(c: u32) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn from_linear(c: f32) -> u32 {
    let c = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (c.clamp(0.0, 1.0) * 255.0).round() as u32
}

fn oklab(c: Rgb) -> [f32; 3] {
    let (r, g, b) = (to_linear(c.0 >> 16 & 0xff), to_linear(c.0 >> 8 & 0xff), to_linear(c.0 & 0xff));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

fn from_oklab([l, a, b]: [f32; 3]) -> Rgb {
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    let r = 4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_;
    let g = -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_;
    let bl = -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_;
    Rgb(from_linear(r) << 16 | from_linear(g) << 8 | from_linear(bl))
}

/// Where a theme comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Builtin,
    Index,
}

/// What the theme picker shows for one theme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeInfo {
    pub id: String,
    pub name: String,
    pub variant: Variant,
    pub source: Source,
}

#[derive(Deserialize)]
struct Head {
    name: String,
    variant: String,
}

fn variant_of(name: &str) -> Variant {
    if name == "light" { Variant::Light } else { Variant::Dark }
}

/// Every available theme, sorted by name: the built-ins, with the index's themes added.
/// An index theme with a built-in's id replaces it. Index files that don't parse are left out.
pub fn list_theme_infos(dir: &Path) -> Vec<ThemeInfo> {
    let mut infos: Vec<ThemeInfo> = BUILTIN
        .iter()
        .filter_map(|(id, text)| {
            let head: Head = serde_json::from_str(text).ok()?;
            Some(ThemeInfo {
                id: (*id).to_owned(),
                name: head.name,
                variant: variant_of(&head.variant),
                source: Source::Builtin,
            })
        })
        .collect();
    for id in list_themes(dir) {
        let Some(head) = fs::read_to_string(dir.join("schemes").join(format!("{id}.json")))
            .ok()
            .and_then(|text| serde_json::from_str::<Head>(&text).ok())
        else {
            continue;
        };
        infos.retain(|info| info.id != id);
        infos.push(ThemeInfo { id, name: head.name, variant: variant_of(&head.variant), source: Source::Index });
    }
    infos.sort_by_key(|i| i.name.to_lowercase());
    infos
}

/// True when the theme index has at least one scheme file. A missing, unreadable or
/// empty index is the same as none.
pub fn index_exists(dir: &Path) -> bool {
    !list_themes(dir).is_empty()
}

#[derive(Deserialize)]
struct SchemeFile {
    name: String,
    variant: String,
    palette: HashMap<String, String>,
    #[serde(default)]
    roles: HashMap<String, String>,
}

#[derive(Deserialize, Default)]
struct Selection {
    dark: Option<String>,
    light: Option<String>,
}

impl Selection {
    fn get(&self, v: Variant) -> Option<&str> {
        match v {
            Variant::Dark => self.dark.as_deref(),
            Variant::Light => self.light.as_deref(),
        }
    }
}

fn from_env_or(var: &str, xdg: &str, home_rel: &[&str], leaf: &str) -> PathBuf {
    if let Some(p) = env::var_os(var) {
        return PathBuf::from(p);
    }
    if let Some(p) = env::var_os(xdg) {
        return PathBuf::from(p).join(leaf);
    }
    let mut p = env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    for part in home_rel {
        p.push(part);
    }
    p.join(leaf)
}

/// `$THEMES_DIR`, else `$XDG_CONFIG_HOME/themes`, else `~/.config/themes`.
pub fn themes_dir() -> PathBuf {
    from_env_or("THEMES_DIR", "XDG_CONFIG_HOME", &[".config"], "themes")
}

/// `$THEMES_STATE_DIR`, else `$XDG_STATE_HOME/themes`, else `~/.local/state/themes`.
pub fn state_dir() -> PathBuf {
    from_env_or("THEMES_STATE_DIR", "XDG_STATE_HOME", &[".local", "state"], "themes")
}

/// Theme ids (file stems in `schemes/`), sorted. Unreadable directory = empty.
pub fn list_themes(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir.join("schemes")) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            name.strip_suffix(".json").map(str::to_owned)
        })
        .collect();
    ids.sort();
    ids
}

fn parse_theme(id: &str, scheme_json: &str, roles_json: &str) -> Result<Theme, String> {
    let scheme: SchemeFile = serde_json::from_str(scheme_json).map_err(|e| format!("{id}.json: {e}"))?;
    let base: HashMap<String, String> = serde_json::from_str(roles_json).map_err(|e| format!("roles.json: {e}"))?;
    let palette: HashMap<String, Rgb> =
        scheme.palette.iter().filter_map(|(k, v)| Some((k.clone(), Rgb::parse(v)?))).collect();
    let mut mapping = base;
    mapping.extend(scheme.roles);
    let mut system = HashMap::new();
    let roles = mapping
        .into_iter()
        .filter_map(|(role, reference)| {
            let color = if reference.starts_with('#') {
                Rgb::parse(&reference)
            } else if let Some(name) = reference.strip_prefix("system:") {
                // Stand-in until the app resolves it: accent is the palette's blue, selection its grey.
                let stand_in = if name == "selection" { "base02" } else { "base0D" };
                system.insert(role.clone(), reference.clone());
                palette.get(stand_in).copied()
            } else {
                palette.get(&reference).copied()
            };
            color.map(|c| (role, c))
        })
        .collect();
    Ok(Theme { id: id.to_owned(), name: scheme.name, variant: variant_of(&scheme.variant), palette, roles, system })
}

/// The index's `roles.json`, or the base mapping when it is missing or not a role map.
fn index_roles(dir: &Path) -> String {
    fs::read_to_string(dir.join("roles.json"))
        .ok()
        .filter(|text| serde_json::from_str::<HashMap<String, String>>(text).is_ok())
        .unwrap_or_else(|| BASE_ROLES.into())
}

fn builtin(id: &str) -> Option<Theme> {
    let (_, text) = BUILTIN.iter().find(|(builtin_id, _)| *builtin_id == id)?;
    Some(parse_theme(id, text, BASE_ROLES).expect("built-in theme parses"))
}

/// Load one theme. The index's file wins over a built-in with the same id; its `roles.json`
/// is merged under the theme's own `roles`. A broken index file is an error, never the
/// built-in of that id.
pub fn load_theme(dir: &Path, id: &str) -> Result<Theme, String> {
    let path = dir.join("schemes").join(format!("{id}.json"));
    match fs::read_to_string(&path) {
        Ok(scheme) => parse_theme(id, &scheme, &index_roles(dir)),
        Err(_) => builtin(id).ok_or_else(|| format!("{id}: not found")),
    }
}

/// The built-in theme for an appearance: Finder Light or Finder Dark.
pub fn fallback_for(appearance: Variant) -> Theme {
    builtin(match appearance {
        Variant::Light => "finder-light",
        Variant::Dark => "finder-dark",
    })
    .expect("Finder themes are built in")
}

/// The built-in theme of `variant`, parsed once. A role a theme lacks comes from
/// the built-in of its own variant, so a light theme never shows a dark color.
fn cached_fallback(variant: Variant) -> &'static Theme {
    static DARK: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();
    static LIGHT: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();
    match variant {
        Variant::Dark => DARK.get_or_init(|| fallback_for(Variant::Dark)),
        Variant::Light => LIGHT.get_or_init(|| fallback_for(Variant::Light)),
    }
}

/// The embedded fallback theme (Finder Dark).
pub fn fallback() -> Theme {
    fallback_for(Variant::Dark)
}

fn read_selection(path: &Path) -> Selection {
    fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// True if `id` is a built-in or an index theme.
pub fn theme_exists(dir: &Path, id: &str) -> bool {
    BUILTIN.iter().any(|(builtin_id, _)| *builtin_id == id) || dir.join("schemes").join(format!("{id}.json")).is_file()
}

/// Which theme to show. `saved` is the app's stored choice: an id, `"system"` or `None`.
/// Order: saved id → (index) `current.json[appearance]` → (index) `defaults.json[appearance]`
/// → Finder Light or Dark.
pub fn resolve_theme_id(saved: Option<&str>, appearance: Variant, dir: &Path, state: &Path) -> String {
    if let Some(s) = saved
        && s != "system"
        && theme_exists(dir, s)
    {
        return s.to_owned();
    }
    if index_exists(dir) {
        for sel in [read_selection(&state.join("current.json")), read_selection(&dir.join("defaults.json"))] {
            if let Some(id) = sel.get(appearance)
                && theme_exists(dir, id)
            {
                return id.to_owned();
            }
        }
    }
    fallback_for(appearance).id
}

/// The saved id when it names a theme that no longer exists (not `"system"`), for the notice.
pub fn missing_saved<'a>(saved: Option<&'a str>, dir: &Path) -> Option<&'a str> {
    saved.filter(|s| *s != "system" && !theme_exists(dir, s))
}

/// Resolve and load in one step, falling back to Finder on any failure.
pub fn current(saved: Option<&str>, appearance: Variant) -> Theme {
    let (dir, state) = (themes_dir(), state_dir());
    load_theme(&dir, &resolve_theme_id(saved, appearance, &dir, &state)).unwrap_or_else(|_| fallback_for(appearance))
}

/// "Apply to all apps": merge `id` into `<state>/current.json` under its variant, atomically.
pub fn set_global(theme: &Theme, state: &Path) -> std::io::Result<()> {
    fs::create_dir_all(state)?;
    let target = state.join("current.json");
    let mut current: serde_json::Map<String, serde_json::Value> =
        fs::read_to_string(&target).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    current.insert(theme.variant.key().into(), theme.id.clone().into());
    // Unique per call: two saves from this process must not share a temp file.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = state.join(format!("current.json.{}.{id}.tmp", std::process::id()));
    let result =
        fs::write(&tmp, serde_json::to_string_pretty(&current)? + "\n").and_then(|()| fs::rename(&tmp, target));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Directories to watch so that every change to the resolved theme fires an event: the
/// nearest existing ancestor of the index (so creating or removing `themes/` is seen), and,
/// when the index exists, the index, `schemes/`, the state dir and the real (de-symlinked)
/// directories behind each loaded file. Edits made through a stow symlink only show up at
/// the real path. Nothing is created.
pub fn watch_dirs() -> BTreeSet<PathBuf> {
    let (dir, state) = (themes_dir(), state_dir());
    let mut dirs = BTreeSet::new();
    if let Some(ancestor) = dir.ancestors().skip(1).find(|a| a.is_dir()) {
        dirs.insert(ancestor.to_path_buf());
    }
    if index_exists(&dir) {
        dirs.extend([dir.clone(), dir.join("schemes")]);
        if let Some(ancestor) = state.ancestors().find(|a| a.is_dir()) {
            dirs.insert(ancestor.to_path_buf());
        }
        let files = ["roles.json".to_owned(), "defaults.json".to_owned()]
            .into_iter()
            .chain(list_themes(&dir).into_iter().map(|id| format!("schemes/{id}.json")));
        for f in files {
            if let Ok(real) = fs::canonicalize(dir.join(f))
                && let Some(parent) = real.parent()
            {
                dirs.insert(parent.to_path_buf());
            }
        }
    }
    dirs.retain(|d| d.is_dir());
    dirs
}

/// Watches the theme index. Every event sends `()`; the receiver debounces (about 100 ms)
/// and reloads, keeping the last good theme if parsing fails. Drop it to stop.
pub struct ThemeWatcher {
    watcher: RecommendedWatcher,
    tx: Sender<()>,
    dirs: BTreeSet<PathBuf>,
}

fn start(tx: &Sender<()>) -> notify::Result<(RecommendedWatcher, BTreeSet<PathBuf>)> {
    let sender = tx.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = sender.send(());
        }
    })?;
    let dirs = watch_dirs();
    for d in &dirs {
        // A folder that vanished since `watch_dirs` looked is not worth failing the rest for.
        let _ = watcher.watch(d, RecursiveMode::NonRecursive);
    }
    Ok((watcher, dirs))
}

/// Watch the theme index (and where it will appear).
pub fn watch(tx: Sender<()>) -> notify::Result<ThemeWatcher> {
    let (watcher, dirs) = start(&tx)?;
    Ok(ThemeWatcher { watcher, tx, dirs })
}

impl ThemeWatcher {
    /// Re-aim the watcher after an event: the index may have appeared, vanished or gained
    /// folders. True if the watched set changed.
    pub fn refresh(&mut self) -> bool {
        let dirs = watch_dirs();
        if dirs == self.dirs {
            return false;
        }
        match start(&self.tx) {
            Ok((watcher, dirs)) => {
                self.watcher = watcher;
                self.dirs = dirs;
                true
            }
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_light_theme_missing_a_role_falls_back_to_the_light_builtin() {
        let mut light = fallback_for(Variant::Light);
        let expected = light.role("fg");
        light.roles.remove("fg");
        assert_eq!(light.role("fg"), expected);
        assert_ne!(expected, fallback_for(Variant::Dark).role("fg"), "light and dark text differ");
    }

    const FIREBOX: &str = include_str!("../tests-data/firebox.json");

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    /// An index with FireBox (dark) and Light One, like a user's `~/.config/themes`.
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::Builder::new().prefix("crow-theme-test-").tempdir().expect("theme fixture");
        let (dir, state) = (root.path().join("themes"), root.path().join("state"));
        write(&dir, "roles.json", BASE_ROLES);
        write(&dir, "schemes/firebox.json", FIREBOX);
        let light = FIREBOX.replace("\"dark\"", "\"light\"").replace("FireBox", "Light One");
        write(&dir, "schemes/light-one.json", &light);
        write(&dir, "defaults.json", r#"{"dark":"firebox","light":"light-one"}"#);
        (root, dir, state)
    }

    /// Empty directories: no index at all.
    fn no_index() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::Builder::new().prefix("crow-theme-test-").tempdir().expect("theme fixture");
        let (dir, state) = (root.path().join("nothing-here"), root.path().join("state"));
        (root, dir, state)
    }

    #[test]
    fn index_theme_roles_override_base_roles() {
        let (_root, dir, _) = fixture();
        let t = load_theme(&dir, "firebox").unwrap();
        // FireBox maps accent to base09 instead of base0D.
        assert_eq!(t.role("accent"), Rgb(0xfe8019));
        // Literal hex override.
        assert_eq!(t.role("error"), Rgb(0xff8678));
        // Untouched role resolves through roles.json.
        assert_eq!(t.role("fg"), Rgb(0xebdbb2));
    }

    #[test]
    fn no_index_lists_exactly_the_builtins() {
        let (_root, dir, state) = no_index();
        let ids: Vec<_> = list_theme_infos(&dir).into_iter().map(|i| i.id).collect();
        assert_eq!(ids.len(), 10);
        for id in BUILTIN.map(|(id, _)| id) {
            assert!(ids.iter().any(|i| i == id), "{id}");
        }
        assert!(list_theme_infos(&dir).iter().all(|i| i.source == Source::Builtin));
        assert!(!index_exists(&dir));
        assert_eq!(resolve_theme_id(None, Variant::Light, &dir, &state), "finder-light");
        assert_eq!(resolve_theme_id(Some("system"), Variant::Dark, &dir, &state), "finder-dark");
    }

    #[test]
    fn system_in_light_appearance_is_finder_light_not_a_dark_theme() {
        let (_root, dir, state) = no_index();
        let theme = load_theme(&dir, &resolve_theme_id(None, Variant::Light, &dir, &state)).unwrap();
        assert_eq!((theme.id.as_str(), theme.variant), ("finder-light", Variant::Light));
    }

    #[test]
    fn empty_or_odd_schemes_folders_are_no_index() {
        for setup in [
            |dir: &Path| fs::create_dir_all(dir.join("schemes")).unwrap(),
            |dir: &Path| write(dir, "schemes", "a file where a folder should be"),
            |dir: &Path| write(dir, "schemes/readme.txt", "not a scheme"),
        ] {
            let (_root, dir, state) = no_index();
            setup(&dir);
            assert!(!index_exists(&dir));
            assert_eq!(list_theme_infos(&dir).len(), 10);
            assert_eq!(resolve_theme_id(None, Variant::Dark, &dir, &state), "finder-dark");
        }
    }

    #[test]
    fn resolution_order_with_an_index() {
        let (_root, dir, state) = fixture();
        assert_eq!(resolve_theme_id(Some("light-one"), Variant::Dark, &dir, &state), "light-one");
        assert_eq!(resolve_theme_id(Some("system"), Variant::Light, &dir, &state), "light-one");
        assert_eq!(resolve_theme_id(Some("missing"), Variant::Dark, &dir, &state), "firebox");
        write(&state, "current.json", r#"{"dark":"light-one"}"#);
        assert_eq!(resolve_theme_id(None, Variant::Dark, &dir, &state), "light-one");
        // A built-in id works as saved and in current.json.
        assert_eq!(resolve_theme_id(Some("gruvbox-dark"), Variant::Light, &dir, &state), "gruvbox-dark");
        write(&state, "current.json", r#"{"light":"catppuccin-latte"}"#);
        assert_eq!(resolve_theme_id(None, Variant::Light, &dir, &state), "catppuccin-latte");
        // A defaults.json naming nothing that exists falls through to Finder.
        write(&state, "current.json", "{}");
        write(&dir, "defaults.json", r#"{"dark":"gone","light":"gone"}"#);
        assert_eq!(resolve_theme_id(None, Variant::Light, &dir, &state), "finder-light");
    }

    #[test]
    fn a_missing_saved_theme_is_reported_once_and_resolves_like_system() {
        let (_root, dir, state) = no_index();
        assert_eq!(missing_saved(Some("gone"), &dir), Some("gone"));
        assert_eq!(missing_saved(Some("system"), &dir), None);
        assert_eq!(missing_saved(Some("gruvbox-dark"), &dir), None);
        assert_eq!(missing_saved(None, &dir), None);
        assert_eq!(resolve_theme_id(Some("gone"), Variant::Light, &dir, &state), "finder-light");
    }

    #[test]
    fn the_index_wins_a_shared_id_and_the_picker_shows_it_once() {
        let (_root, dir, _) = fixture();
        let own = FIREBOX.replace("FireBox", "My Solarized");
        write(&dir, "schemes/solarized-dark.json", &own);
        let infos = list_theme_infos(&dir);
        let shared: Vec<_> = infos.iter().filter(|i| i.id == "solarized-dark").collect();
        assert_eq!(shared.len(), 1);
        assert_eq!((shared[0].name.as_str(), shared[0].source), ("My Solarized", Source::Index));
        assert_eq!(load_theme(&dir, "solarized-dark").unwrap().name, "My Solarized");
        assert_eq!(infos.len(), 12);
    }

    #[test]
    fn broken_scheme_is_an_error_not_a_panic_and_not_listed() {
        let (_root, dir, _) = fixture();
        write(&dir, "schemes/broken.json", "{ not json");
        let error = load_theme(&dir, "broken").unwrap_err();
        assert!(error.contains("broken.json"), "{error}");
        assert!(list_themes(&dir).contains(&"broken".to_owned()));
        assert!(!list_theme_infos(&dir).iter().any(|i| i.id == "broken"));
        // A broken file shadowing a built-in id is an error too, not a silent built-in.
        write(&dir, "schemes/finder-dark.json", "{");
        assert!(load_theme(&dir, "finder-dark").is_err());
    }

    #[test]
    fn broken_roles_json_uses_the_base_mapping() {
        let (_root, dir, _) = fixture();
        write(&dir, "roles.json", "[1, 2");
        assert_eq!(load_theme(&dir, "light-one").unwrap().role("fg"), Rgb(0xebdbb2));
        write(&dir, "roles.json", r#"{"fg": 5}"#);
        assert!(load_theme(&dir, "light-one").is_ok());
    }

    #[test]
    fn current_json_may_name_a_builtin() {
        let (_root, dir, state) = no_index();
        write(&state, "current.json", r#"{"dark":"gruvbox-dark"}"#);
        // No index: current.json is ignored, the built-in for the appearance wins.
        assert_eq!(resolve_theme_id(None, Variant::Dark, &dir, &state), "finder-dark");
    }

    #[test]
    fn system_references_resolve_through_the_resolver() {
        let t = fallback_for(Variant::Light);
        let stand_in = t.role("accent");
        assert_eq!(stand_in, Rgb(0x007aff));
        assert_eq!(t.system_refs().count(), 3);
        assert_eq!(t.role("selection"), Rgb(0xdcdcdc), "the unfocused selection grey is not the accent");
        let fake = |reference: &str| match reference {
            "system:accent" => Some(Rgb(0xf74f9e)),
            _ => None,
        };
        let resolved = t.resolve_system(&fake);
        assert_eq!(resolved.role("accent"), Rgb(0xf74f9e));
        assert_eq!(resolved.role("selection"), Rgb(0xdcdcdc), "unknown reference keeps its stand-in");
        assert_eq!(resolved.role("link"), Rgb(0xf74f9e));
        // Unknown references keep the stand-in; other themes have none.
        let unknown = fallback_for(Variant::Dark).resolve_system(&|_| None);
        assert_eq!(unknown.role("accent"), Rgb(0x0091ff));
        assert_eq!(builtin("gruvbox-dark").unwrap().system_refs().count(), 0);
    }

    #[test]
    fn every_builtin_parses_with_all_roles_and_the_ids_match() {
        let infos = list_theme_infos(Path::new("/nonexistent"));
        for (id, _) in BUILTIN {
            let t = builtin(id).unwrap();
            for role in ROLES {
                assert!(t.roles.contains_key(role), "{id} lacks {role}");
            }
            for slot in 0..=0x17 {
                assert!(t.slot(&format!("base{slot:02X}")).is_some(), "{id} lacks base{slot:02X}");
            }
            assert!(infos.iter().any(|i| i.id == id));
        }
        let variants: Vec<_> = infos.iter().map(|i| i.variant).collect();
        assert_eq!(variants.iter().filter(|v| **v == Variant::Light).count(), 5);
    }

    /// WCAG relative luminance contrast ratio.
    fn contrast(a: Rgb, b: Rgb) -> f32 {
        let lum = |c: Rgb| {
            0.2126 * to_linear(c.0 >> 16 & 0xff) + 0.7152 * to_linear(c.0 >> 8 & 0xff) + 0.0722 * to_linear(c.0 & 0xff)
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// Pairs below the threshold on purpose, with why. `(theme id, foreground role, background role)`.
    /// Finder's accent is the user's system accent colour, resolved at run time;
    /// the stand-in only approximates the default blue.
    const CONTRAST_EXCEPTIONS: &[(&str, &str, &str)] =
        &[("finder-light", "on-accent", "accent"), ("finder-dark", "on-accent", "accent")];

    #[test]
    fn builtin_contrast_meets_wcag_aa() {
        // (foreground, background, minimum): text 4.5, UI 3.
        let pairs = [
            ("fg", "bg", 4.5),
            ("fg", "bg-inset", 4.5),
            ("fg-muted", "bg", 4.5),
            ("fg-muted", "bg-inset", 4.5),
            ("on-accent", "accent", 4.5),
            ("accent", "bg", 3.0),
            ("error", "bg", 4.5),
            ("warning", "bg", 3.0),
            ("success", "bg", 3.0),
            ("info", "bg", 3.0),
            ("fg", "selection", 4.5),
        ];
        let mut failures = Vec::new();
        for (id, _) in BUILTIN {
            let t = builtin(id).unwrap();
            for (fg, bg, min) in pairs {
                let ratio = contrast(t.role(fg), t.role(bg));
                if ratio < min && !CONTRAST_EXCEPTIONS.contains(&(id, fg, bg)) {
                    failures.push(format!("{id}: {fg} on {bg} is {ratio:.2}:1, needs {min}"));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn mixing_runs_from_one_theme_to_the_other() {
        let (_root, dir, _) = fixture();
        let (from, to) = (fallback(), load_theme(&dir, "light-one").unwrap());
        let mut to = to;
        to.roles.insert("bg".into(), Rgb(0xffffff));
        assert!(from.mix(&to, 0.0).same_colors(&from));
        assert!(from.mix(&to, 1.0).same_colors(&to));
        assert_eq!(from.mix(&to, 0.5).id, "light-one");
        let mid = Rgb(0x000000).mix(Rgb(0xffffff), 0.5);
        // OKLab's L = 0.5 is #636363, darker than sRGB's halfway #808080.
        assert_eq!(mid, Rgb(0x636363));
        assert_eq!(Rgb(0x123456).mix(Rgb(0x123456), 0.3), Rgb(0x123456));
        assert_eq!(Rgb(0xfe8019).mix(Rgb(0x000000), 0.0), Rgb(0xfe8019), "round trip is exact");
    }

    #[test]
    fn theme_infos_have_names_variants_and_sources() {
        let (_root, dir, _) = fixture();
        write(&dir, "schemes/broken.json", "{");
        let infos = list_theme_infos(&dir);
        let find = |id: &str| infos.iter().find(|i| i.id == id).map(|i| (i.name.as_str(), i.variant, i.source));
        assert_eq!(find("firebox"), Some(("FireBox", Variant::Dark, Source::Index)));
        assert_eq!(find("light-one"), Some(("Light One", Variant::Light, Source::Index)));
        assert_eq!(find("finder-light"), Some(("Finder Light", Variant::Light, Source::Builtin)));
    }

    #[test]
    fn watching_creates_nothing() {
        let (_root, dir, state) = no_index();
        assert!(!dir.exists() && !state.exists());
        // `watch_dirs` reads the process environment, so only the pure parts are checked here:
        // an absent index and state dir stay absent after listing and resolving.
        let _ = (list_theme_infos(&dir), resolve_theme_id(None, Variant::Dark, &dir, &state));
        assert!(!dir.exists() && !state.exists());
    }

    #[test]
    fn unknown_role_falls_back() {
        let t = fallback();
        assert_ne!(t.role("surface"), Rgb(0xff00ff));
    }
}
