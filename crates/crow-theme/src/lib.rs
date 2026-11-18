//! Reader for the global theme index (`~/.config/themes`).
//!
//! Implements the reader protocol: Base24 palettes plus a fixed set of semantic
//! roles. Crow only ever reads the theme directory; the global "apply to all
//! apps" selection lives in the state directory and is written atomically.

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

/// The theme compiled into the binary, used when the index is missing or broken.
const FALLBACK_SCHEME: &str = include_str!("fallback.json");
const FALLBACK_ROLES: &str = r##"{
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
}

impl Theme {
    /// The color for a role. Roles missing from an older index fall back to the
    /// embedded theme, so this never fails.
    pub fn role(&self, role: &str) -> Rgb {
        self.roles.get(role).copied().or_else(|| fallback().roles.get(role).copied()).unwrap_or(Rgb(0xff00ff))
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

/// What the theme picker shows for one theme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeInfo {
    pub id: String,
    pub name: String,
    pub variant: Variant,
}

/// Every theme in the index with its display name and variant, sorted by name.
/// Files that don't parse are left out.
pub fn list_theme_infos(dir: &Path) -> Vec<ThemeInfo> {
    #[derive(Deserialize)]
    struct Head {
        name: String,
        variant: String,
    }
    let mut infos: Vec<ThemeInfo> = list_themes(dir)
        .into_iter()
        .filter_map(|id| {
            let text = fs::read_to_string(dir.join("schemes").join(format!("{id}.json"))).ok()?;
            let head: Head = serde_json::from_str(&text).ok()?;
            let variant = if head.variant == "light" { Variant::Light } else { Variant::Dark };
            Some(ThemeInfo { id, name: head.name, variant })
        })
        .collect();
    infos.sort_by_key(|i| i.name.to_lowercase());
    infos
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
    let scheme: SchemeFile = serde_json::from_str(scheme_json).map_err(|e| format!("{id}: {e}"))?;
    let base: HashMap<String, String> = serde_json::from_str(roles_json).map_err(|e| format!("roles.json: {e}"))?;
    let palette: HashMap<String, Rgb> =
        scheme.palette.iter().filter_map(|(k, v)| Some((k.clone(), Rgb::parse(v)?))).collect();
    let mut mapping = base;
    mapping.extend(scheme.roles);
    let roles = mapping
        .into_iter()
        .filter_map(|(role, reference)| {
            let color =
                if reference.starts_with('#') { Rgb::parse(&reference) } else { palette.get(&reference).copied() };
            color.map(|c| (role, c))
        })
        .collect();
    let variant = if scheme.variant == "light" { Variant::Light } else { Variant::Dark };
    Ok(Theme { id: id.to_owned(), name: scheme.name, variant, palette, roles })
}

/// Load one theme: `roles.json` merged with the theme's own `roles` (theme wins).
pub fn load_theme(dir: &Path, id: &str) -> Result<Theme, String> {
    let scheme =
        fs::read_to_string(dir.join("schemes").join(format!("{id}.json"))).map_err(|e| format!("{id}: {e}"))?;
    let roles = fs::read_to_string(dir.join("roles.json")).unwrap_or_else(|_| FALLBACK_ROLES.into());
    parse_theme(id, &scheme, &roles)
}

/// The embedded fallback theme (FireBox).
pub fn fallback() -> Theme {
    parse_theme("firebox", FALLBACK_SCHEME, FALLBACK_ROLES).expect("embedded theme parses")
}

fn read_selection(path: &Path) -> Selection {
    fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Which theme to show. `saved` is the app's stored choice: an id, `"system"` or `None`.
/// Order: saved id → `current.json[appearance]` → `defaults.json[appearance]` → first theme.
pub fn resolve_theme_id(saved: Option<&str>, appearance: Variant, dir: &Path, state: &Path) -> Option<String> {
    let ids = list_themes(dir);
    if let Some(s) = saved
        && s != "system"
        && ids.iter().any(|i| i == s)
    {
        return Some(s.to_owned());
    }
    for sel in [read_selection(&state.join("current.json")), read_selection(&dir.join("defaults.json"))] {
        if let Some(id) = sel.get(appearance)
            && ids.iter().any(|i| i == id)
        {
            return Some(id.to_owned());
        }
    }
    ids.into_iter().next()
}

/// Resolve and load in one step, falling back to the embedded theme on any failure.
pub fn current(saved: Option<&str>, appearance: Variant) -> Theme {
    let (dir, state) = (themes_dir(), state_dir());
    resolve_theme_id(saved, appearance, &dir, &state).and_then(|id| load_theme(&dir, &id).ok()).unwrap_or_else(fallback)
}

/// "Apply to all apps": merge `id` into `<state>/current.json` under its variant, atomically.
pub fn set_global(theme: &Theme, state: &Path) -> std::io::Result<()> {
    fs::create_dir_all(state)?;
    let target = state.join("current.json");
    let mut current: serde_json::Map<String, serde_json::Value> =
        fs::read_to_string(&target).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    current.insert(theme.variant.key().into(), theme.id.clone().into());
    let tmp = state.join(format!("current.json.{}.tmp", std::process::id()));
    fs::write(&tmp, serde_json::to_string_pretty(&current)? + "\n")?;
    fs::rename(tmp, target)
}

/// Directories to watch so that every change to the resolved theme fires an
/// event: the index, `schemes/`, the state dir, and the real (de-symlinked)
/// directories behind each loaded file. Edits made through a stow symlink only
/// show up at the real path.
pub fn watch_dirs() -> BTreeSet<PathBuf> {
    let (dir, state) = (themes_dir(), state_dir());
    // Created so a later first write of current.json is seen; failure just means no state dir to watch.
    fs::create_dir_all(&state).ok();
    let mut dirs: BTreeSet<PathBuf> = [dir.clone(), dir.join("schemes"), state].into_iter().collect();
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
    dirs.retain(|d| d.is_dir());
    dirs
}

/// Watch the theme index. Every event sends `()`; the receiver debounces
/// (about 100 ms) and reloads, keeping the last good theme if parsing fails.
/// Drop the returned watcher to stop. Recreate it if `watch_dirs()` changes.
pub fn watch(tx: Sender<()>) -> notify::Result<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = tx.send(());
        }
    })?;
    for d in watch_dirs() {
        watcher.watch(&d, RecursiveMode::NonRecursive)?;
    }
    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::Builder::new().prefix("crow-theme-test-").tempdir().expect("theme fixture");
        let (dir, state) = (root.path().join("themes"), root.path().join("state"));
        write(&dir, "roles.json", FALLBACK_ROLES);
        write(&dir, "schemes/firebox.json", FALLBACK_SCHEME);
        let light = FALLBACK_SCHEME.replace("\"dark\"", "\"light\"").replace("FireBox", "Light One");
        write(&dir, "schemes/light-one.json", &light);
        write(&dir, "defaults.json", r#"{"dark":"firebox","light":"light-one"}"#);
        (root, dir, state)
    }

    #[test]
    fn theme_roles_override_base_roles() {
        let t = fallback();
        // FireBox maps accent to base09 instead of base0D.
        assert_eq!(t.role("accent"), Rgb(0xfe8019));
        // Literal hex override.
        assert_eq!(t.role("error"), Rgb(0xff8678));
        // Untouched role resolves through roles.json.
        assert_eq!(t.role("fg"), Rgb(0xebdbb2));
    }

    #[test]
    fn resolution_order() {
        let (_root, dir, state) = fixture();
        assert_eq!(resolve_theme_id(Some("light-one"), Variant::Dark, &dir, &state).as_deref(), Some("light-one"));
        assert_eq!(resolve_theme_id(Some("system"), Variant::Light, &dir, &state).as_deref(), Some("light-one"));
        assert_eq!(resolve_theme_id(Some("missing"), Variant::Dark, &dir, &state).as_deref(), Some("firebox"));
        write(&state, "current.json", r#"{"dark":"light-one"}"#);
        assert_eq!(resolve_theme_id(None, Variant::Dark, &dir, &state).as_deref(), Some("light-one"));
    }

    #[test]
    fn broken_scheme_is_an_error_not_a_panic() {
        let (_root, dir, _) = fixture();
        write(&dir, "schemes/broken.json", "{ not json");
        assert!(load_theme(&dir, "broken").is_err());
        assert!(list_themes(&dir).contains(&"broken".to_owned()));
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
    fn theme_infos_have_names_and_variants() {
        let (_root, dir, _) = fixture();
        write(&dir, "schemes/broken.json", "{");
        let infos = list_theme_infos(&dir);
        let names: Vec<_> = infos.iter().map(|i| (i.name.as_str(), i.variant)).collect();
        assert_eq!(names, [("FireBox", Variant::Dark), ("Light One", Variant::Light)]);
    }

    #[test]
    fn unknown_role_falls_back() {
        let t = fallback();
        assert_ne!(t.role("surface"), Rgb(0xff00ff));
    }
}
