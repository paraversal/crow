//! `crow --component-shot NAME --theme ID --out FILE.png`: one Crow component, cut out of
//! the real window and drawn with fixture data, for the website's shots.
//!
//! The real `CrowWindow` is opened in GPUI's headless test context (no NSWindow, the
//! Metal headless renderer, the Mac text system), driven to the wanted state, and the
//! component's bounds, read from the element tree by `debug_selector`, are cut out of
//! the frame and padded with the crop's own background. Nothing is re-implemented, so a
//! change to a native component changes its shot. It needs a debug build (only those
//! record selectors) with the `component-shots` feature:
//!
//! ```text
//! cargo run -p crow-app --features component-shots -- --component-shot git-rows --theme finder-light --out rows.png
//! ```
//!
//! `scripts/component-shots.sh` renders every component in a light and a dark theme.
//! Fixtures live under a scratch folder that is removed afterwards; the config, state
//! and home folders are the scratch ones, never the user's.

// Tests call `render`; only the `component-shots` binary calls `run`.
#![cfg_attr(not(feature = "component-shots"), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Bounds, HeadlessAppContext, Keystroke, Pixels, WindowHandle};
use image::{Rgba, RgbaImage};

use crate::window::CrowWindow;
use crate::{effects, extensions, icons, operations, plugins, ui};

/// The render scale of GPUI's headless test window.
const SCALE: f32 = 2.0;
/// Padding around a crop, in points.
const PAD: f32 = 20.0;
/// The component names, in the order the script renders them.
pub const COMPONENTS: [&str; 5] = ["git-rows", "git-sidebar", "git-diff", "compare-gallery", "plugin-surfaces"];

const ICON_PACK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/examples/icon-pack");

/// A scratch folder that is removed on drop.
pub struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self, String> {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("crow-component-shot-{}-{n}", std::process::id()));
        // io-ok: developer tool
        std::fs::remove_dir_all(&dir).ok();
        // io-ok: developer tool
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        // Canonical, so a symlinked $TMPDIR cannot change what the window reads.
        // io-ok: developer tool
        Ok(Self(dir.canonicalize().map_err(|e| e.to_string())?))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // io-ok: developer tool
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// What a component adds to the standard settings.
#[derive(Default)]
struct Extra {
    settings: String,
    views: String,
    sidebar: String,
    sidebar_width: u32,
    plugin: bool,
    grant_git: bool,
}

/// Which pixels of the window a shot keeps: the union of these elements and of
/// `prefix0`, `prefix1`, ... as far as they exist.
struct Crop {
    selectors: Vec<String>,
    numbered: Option<&'static str>,
}

impl Crop {
    fn of(selectors: &[&str]) -> Self {
        Self { selectors: selectors.iter().map(|s| (*s).to_owned()).collect(), numbered: None }
    }
}

struct Plan {
    window: (f32, f32),
    folder: PathBuf,
    extra: Extra,
    /// Space-separated keystrokes sent once the folder has loaded.
    keys: &'static str,
    git_diff: bool,
    crop: Crop,
    /// Points added to the crop's top, right, bottom and left before padding.
    grow: [f32; 4],
}

fn io<T>(result: std::io::Result<T>) -> Result<T, String> {
    result.map_err(|e| e.to_string())
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        // io-ok: developer tool
        io(std::fs::create_dir_all(parent))?;
    }
    // io-ok: developer tool
    io(std::fs::write(path, text))
}

fn git(repo: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Ines Calder")
        .env("GIT_AUTHOR_EMAIL", "ines@example.com")
        .env("GIT_COMMITTER_NAME", "Ines Calder")
        .env("GIT_COMMITTER_EMAIL", "ines@example.com")
        .env("GIT_AUTHOR_DATE", "2026-09-14T10:00:00")
        .env("GIT_COMMITTER_DATE", "2026-09-14T10:00:00")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("git {args:?}: {}", String::from_utf8_lossy(&output.stderr)))
    }
}

/// A fixed modification time (local), so a Modified column never says "Today".
fn touch(path: &Path, stamp: &str) -> Result<(), String> {
    let status = Command::new("touch").arg("-t").arg(stamp).arg(path).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("touch {}", path.display())) }
}

const TIDES_RS: &str = "\
//! Next high water for a harbour.

pub struct Tide {
    pub cm: u32,
    pub in_min: u32,
}

const OFFSET_MIN: u32 = 14;

pub fn next_high(t: &[Tide]) -> Option<&Tide> {
    t.iter().max_by_key(|x| x.cm)
}

pub fn arrives_in(t: &Tide) -> u32 {
    t.in_min
}
";

const TIDES_RS_EDITED: &str = "\
//! Next high water for a harbour.

pub struct Tide {
    pub cm: u32,
    pub in_min: u32,
}

const OFFSET_MIN: u32 = 22;

/// The highest tide still to come.
pub fn next_high(t: &[Tide]) -> Option<&Tide> {
    t.iter()
        .filter(|x| x.in_min > 0)
        .max_by_key(|x| x.cm)
}

pub fn arrives_in(t: &Tide) -> u32 {
    t.in_min + OFFSET_MIN
}
";

/// A repository `root/name` on `branch` with these committed files, then these working-tree
/// edits (a path and its new text; a new path is untracked), all with fixed dates.
fn make_repo(
    root: &Path,
    name: &str,
    branch: &str,
    committed: &[(&str, &str)],
    edits: &[(&str, &str)],
) -> Result<PathBuf, String> {
    let repo = root.join(name);
    // io-ok: developer tool
    io(std::fs::create_dir_all(&repo))?;
    git(&repo, &["init", "-q", "-b", "main"])?;
    for (path, text) in committed {
        write(&repo.join(path), text)?;
        touch(&repo.join(path), "202609141000")?;
    }
    git(&repo, &["add", "."])?;
    git(&repo, &["commit", "-q", "-m", "Start"])?;
    if branch != "main" {
        git(&repo, &["checkout", "-q", "-b", branch])?;
    }
    for (path, text) in edits {
        write(&repo.join(path), text)?;
        touch(&repo.join(path), "202611031620")?;
    }
    Ok(repo)
}

/// `tidepool`: three entries in `src` with a modified, an untracked and an ignored file.
fn tidepool(projects: &Path) -> Result<PathBuf, String> {
    let repo = make_repo(
        projects,
        "tidepool",
        "main",
        &[("src/tides.rs", TIDES_RS), (".gitignore", "*.log\n"), ("Cargo.toml", "[package]\nname = \"tidepool\"\n")],
        &[
            ("src/tides.rs", TIDES_RS_EDITED),
            ("src/harbor.toml", "[harbour]\nname = \"Reine\"\noffset_min = 22\n"),
            ("src/scratch.log", "14:02 table loaded\n"),
        ],
    )?;
    touch(&repo.join("src/scratch.log"), "202611011200")?;
    Ok(repo)
}

/// Four projects with different branches and numbers of changes.
fn projects(projects_dir: &Path) -> Result<(), String> {
    tidepool(projects_dir)?;
    let lines = |n: usize| (0..n).map(|i| format!("line {i}\n")).collect::<String>();
    let files: Vec<(String, String)> = (0..5).map(|i| (format!("tables/spring-{i}.csv"), lines(3))).collect();
    let committed: Vec<(&str, &str)> = files.iter().map(|(p, t)| (p.as_str(), t.as_str())).collect();
    let edited: Vec<(String, String)> = (0..5).map(|i| (format!("tables/spring-{i}.csv"), lines(5))).collect();
    let edits: Vec<(&str, &str)> = edited.iter().map(|(p, t)| (p.as_str(), t.as_str())).collect();
    make_repo(projects_dir, "lanternfish", "feature/spring-tables", &committed, &edits)?;
    make_repo(projects_dir, "kelp", "main", &[("README.md", "# kelp\n")], &[])?;
    make_repo(
        projects_dir,
        "reef-watch",
        "release-2",
        &[("watch.py", "print('ok')\n")],
        &[("watch.py", "print('ok!')\n")],
    )?;
    Ok(())
}

/// A soft landscape, 3 x 2, generated: a sky gradient, a sun, layered ridges and still water.
fn landscape(sky: [[u8; 3]; 2], sun: [u8; 3], sun_at: (f32, f32), ridge: [u8; 3], dusk: bool) -> RgbaImage {
    let (w, h) = (1200u32, 800u32);
    let horizon = h as f32 * 0.62;
    let mix = |a: [u8; 3], b: [u8; 3], t: f32| -> [f32; 3] {
        let t = t.clamp(0.0, 1.0);
        [0, 1, 2].map(|i| a[i] as f32 * (1.0 - t) + b[i] as f32 * t)
    };
    let ridge_y = |x: f32, layer: f32| -> f32 {
        let u = x / w as f32;
        horizon
            - 40.0 * layer
            - 70.0 * ((u * (2.1 + layer) + layer).sin() * 0.5 + 0.5) * (0.6 + 0.2 * layer)
            - 28.0 * ((u * 7.3 + layer * 2.0).sin() * 0.5 + 0.5)
    };
    let sun_px = (sun_at.0 * w as f32, sun_at.1 * h as f32);
    RgbaImage::from_fn(w, h, |x, y| {
        let (fx, fy) = (x as f32, y as f32);
        let sky_t = (fy / horizon).clamp(0.0, 1.0);
        let mut colour = mix(sky[0], sky[1], sky_t.powf(1.4));
        let glow = (-(((fx - sun_px.0).powi(2) + (fy - sun_px.1).powi(2)).sqrt() / 190.0)).exp();
        colour = mix_f(colour, sun, glow * 0.55);
        let disc = ((fx - sun_px.0).powi(2) + (fy - sun_px.1).powi(2)).sqrt();
        if disc < 34.0 {
            colour = mix_f(colour, [255, 250, 235], (34.0 - disc).clamp(0.0, 3.0) / 3.0);
        }
        if fy >= horizon {
            // Water: the sky's reflection, darker, with faint ripples.
            let t = ((fy - horizon) / (h as f32 - horizon)).clamp(0.0, 1.0);
            let mirrored = mix(sky[1], sky[0], t.powf(0.7));
            let ripple = ((fy * 0.9 + (fx * 0.013).sin() * 6.0).sin() * 0.5 + 0.5) * 0.06;
            colour = mix_f(mirrored, ridge, 0.35 + t * 0.4 - ripple);
            let reflected = (-(((fx - sun_px.0).abs()) / (60.0 + t * 80.0))).exp() * (1.0 - t) * 0.5;
            colour = mix_f(colour, sun, reflected);
        }
        for (layer, depth) in [(2.0f32, 0.35f32), (1.0, 0.6), (0.0, 0.85)] {
            if fy < horizon + 2.0 && fy > ridge_y(fx, layer) {
                let tone = if dusk { [18, 20, 36] } else { [34, 52, 66] };
                colour = mix_f(ridge.map(f32::from), tone, depth);
            }
        }
        // A trace of dither keeps the gradients from banding; it is a function of the position.
        let n = (((x.wrapping_mul(73856093)) ^ (y.wrapping_mul(19349663))) % 256) as f32 / 255.0 - 0.5;
        let [r, g, b] = [0, 1, 2].map(|i| (colour[i] + n * 1.5).clamp(0.0, 255.0) as u8);
        Rgba([r, g, b, 255])
    })
}

fn mix_f(a: [f32; 3], b: [u8; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| a[i] * (1.0 - t) + b[i] as f32 * t)
}

fn plan(name: &str, scratch: &Scratch) -> Result<Plan, String> {
    let projects_dir = scratch.0.join("Projects");
    let no_sections = |extra: &str| {
        format!("[recent]\nshow = false\n[locations]\nshow = false\n[section.tags]\nshow = false\n{extra}")
    };
    let list = "view = \"list\"\ncolumns = [\"size\", \"git.lines-changed\"]\n";
    match name {
        "git-rows" => {
            let repo = tidepool(&projects_dir)?;
            Ok(Plan {
                window: (700.0, 360.0),
                folder: repo.join("src"),
                extra: Extra {
                    settings: "show_preview = false\n".into(),
                    views: list.into(),
                    sidebar: no_sections(""),
                    grant_git: true,
                    ..Default::default()
                },
                keys: "",
                git_diff: false,
                crop: Crop::of(&["table-header", "row-harbor.toml", "row-scratch.log", "row-tides.rs"]),
                grow: [0.0; 4],
            })
        }
        "git-sidebar" => {
            projects(&projects_dir)?;
            Ok(Plan {
                window: (700.0, 420.0),
                folder: projects_dir.clone(),
                extra: Extra {
                    settings: "show_preview = false\n".into(),
                    views: list.into(),
                    sidebar: no_sections(&format!(
                        "[projects]\nroot = \"{}\"\nshow_root = false\n",
                        projects_dir.display()
                    )),
                    grant_git: true,
                    ..Default::default()
                },
                keys: "",
                git_diff: false,
                crop: Crop { selectors: Vec::new(), numbered: Some("sidebar-") },
                grow: [34.0, 0.0, 6.0, 0.0],
            })
        }
        "git-diff" => {
            let repo = tidepool(&projects_dir)?;
            Ok(Plan {
                window: (1100.0, 410.0),
                folder: repo.join("src"),
                extra: Extra {
                    settings: "show_preview = true\n".into(),
                    views: list.into(),
                    sidebar: no_sections(""),
                    grant_git: true,
                    ..Default::default()
                },
                keys: "down down",
                git_diff: true,
                crop: Crop::of(&["info-tabs", "info-body"]),
                grow: [0.0; 4],
            })
        }
        "compare-gallery" => {
            let folder = scratch.0.join("Pictures").join("Reine");
            // io-ok: developer tool
            io(std::fs::create_dir_all(&folder))?;
            let dawn =
                landscape([[120, 126, 190], [250, 190, 150]], [255, 205, 150], (0.68, 0.5), [120, 130, 150], false);
            let dusk = landscape([[22, 26, 70], [236, 120, 90]], [255, 140, 90], (0.3, 0.55), [60, 50, 80], true);
            io(dawn.save(folder.join("Reine at dawn.png")).map_err(std::io::Error::other))?;
            io(dusk.save(folder.join("Reine at dusk.png")).map_err(std::io::Error::other))?;
            for file in ["Reine at dawn.png", "Reine at dusk.png"] {
                touch(&folder.join(file), "202603081030")?;
            }
            Ok(Plan {
                window: (1240.0, 340.0),
                folder,
                extra: Extra {
                    settings: "show_preview = false\n".into(),
                    views: "view = \"gallery\"\n".into(),
                    sidebar: no_sections(""),
                    ..Default::default()
                },
                keys: "shift-down",
                git_diff: false,
                crop: Crop::of(&["gallery-preview"]),
                grow: [0.0; 4],
            })
        }
        "plugin-surfaces" => {
            let folder = scratch.0.join("Pictures").join("2026-03 Lofoten");
            for dir in ["Selects", "Edits", "Exports"] {
                // io-ok: developer tool
                io(std::fs::create_dir_all(folder.join(dir)))?;
            }
            for dir in ["Selects", "Edits", "Exports"] {
                touch(&folder.join(dir), "202603081030")?;
            }
            for file in [
                "Reine sunrise.dng",
                "Reine sunrise.jpg",
                "Hamnoy fjord.dng",
                "Hamnoy fjord.jpg",
                "Sakrisoy reject.jpg",
            ] {
                write(&folder.join(file), "x")?;
                touch(&folder.join(file), "202603081030")?;
            }
            Ok(Plan {
                window: (760.0, 380.0),
                folder,
                extra: Extra {
                    settings: "show_preview = false\n".into(),
                    views: "view = \"list\"\ncolumns = [\"example.shoot-notes.rating\"]\n".into(),
                    sidebar: no_sections("[sections]\norder = [\"example.shoot-notes.stages\"]\n"),
                    plugin: true,
                    sidebar_width: 250,
                    ..Default::default()
                },
                keys: "",
                git_diff: false,
                crop: Crop { numbered: Some("sidebar-"), ..Crop::of(&["table-header", "row-Sakrisoy reject.jpg"]) },
                grow: [0.0; 4],
            })
        }
        other => Err(format!("unknown component \"{other}\" (one of {})", COMPONENTS.join(", "))),
    }
}

/// A headless Crow window on a fixture folder.
struct Session {
    cx: HeadlessAppContext,
    window: WindowHandle<CrowWindow>,
}

impl Session {
    fn open(theme: &str, plan: &Plan, scratch: &Scratch) -> Result<Self, String> {
        let config = scratch.0.join("config");
        let state = scratch.0.join("state");
        let home = scratch.0.join("home");
        let trash = scratch.0.join("trash");
        for dir in [&state, &home, &trash] {
            // io-ok: developer tool
            io(std::fs::create_dir_all(dir))?;
        }
        write(&state.join(crow_core::ONBOARDING_FILE), "{\"tips_choice\": true}\n")?;
        let extra = &plan.extra;
        write(
            &config.join("settings.toml"),
            &format!(
                "theme = \"{theme}\"\nkeymap = \"finder\"\nhints = \"off\"\nmotion = \"off\"\nicon_pack = \"lucide\"\n{}\n[window]\nwidth = {}\nheight = {}\nsidebar_width = {}\n",
                extra.settings,
                plan.window.0 as u32,
                plan.window.1 as u32,
                if extra.sidebar_width == 0 { 300 } else { extra.sidebar_width }
            ),
        )?;
        write(&config.join("views.toml"), &format!("[defaults]\n{}\n", extra.views))?;
        write(&config.join("sidebar.toml"), &extra.sidebar)?;
        copy_dir(Path::new(ICON_PACK), &config.join("icon-packs/lucide"))?;
        if extra.grant_git {
            crate::bundled_grant::grant_bundled("git", &config)?;
        }
        if extra.plugin {
            let dir = config.join("plugins/shoot-notes");
            write(&dir.join("plugin.toml"), include_str!("../assets/component-shots/shoot-notes/plugin.toml"))?;
            write(&dir.join("main.rhai"), include_str!("../assets/component-shots/shoot-notes/main.rhai"))?;
            let package = crow_plugin::load_package(&dir, &crow_plugin::Limits::default())
                .map_err(|e| format!("example plugin: {:?}: {}", e.kind, e.message))?;
            crow_plugin::grants::approve(&config.join("plugins.toml"), &package.manifest.id, &package)?;
        }
        let text_system = Arc::new(gpui_apple::AppleTextSystem::new());
        let mut cx = HeadlessAppContext::with_platform(text_system, Arc::new(icons::Assets), || {
            gpui_platform::current_headless_renderer()
        });
        let folder = plan.folder.clone();
        cx.update(|cx: &mut App| {
            effects::install(cx, Arc::new(effects::RecordingEffects::without_file_urls()));
            ui::init_at(cx, config.clone(), state.clone(), home.clone(), state.join("cache"));
            extensions::init(cx);
            operations::init(cx, Arc::new(operations::FolderTrash(trash)));
            plugins::init(cx);
            crate::icon_packs::reload(cx);
            crate::keymap::install(cx);
            cx.set_global(crate::window::LaunchFolder(folder.clone()));
        });
        let window = cx.update(|cx| crate::window::open(folder, false, None, cx)).ok_or("could not open the window")?;
        let mut session = Self { cx, window };
        session.pump(600);
        Ok(session)
    }

    /// Run the app for about `ms` of real time: tasks, folder loads on their threads, the Git scan.
    fn pump(&mut self, ms: u64) {
        for _ in 0..ms / 10 {
            self.cx.advance_clock(Duration::from_millis(20));
            self.cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn keys(&mut self, keys: &'static str) -> Result<(), String> {
        for key in keys.split_whitespace() {
            let keystroke = Keystroke::parse(key).map_err(|e| format!("{key}: {e}"))?;
            self.cx
                .update_window(self.window.into(), |_, window, cx| window.dispatch_keystroke(keystroke, cx))
                .map_err(|e| e.to_string())?;
            self.pump(120);
        }
        Ok(())
    }

    fn show_git_diff(&mut self) -> Result<(), String> {
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.dispatch_action(Box::new(crate::actions::ShowGitDiff), cx)
            })
            .map_err(|e| e.to_string())?;
        self.pump(300);
        Ok(())
    }

    fn bounds(&mut self, selector: &str) -> Option<Bounds<Pixels>> {
        self.cx.update_window(self.window.into(), |_, window, _| window.debug_bounds(selector)).ok().flatten()
    }

    fn image(&mut self) -> Result<RgbaImage, String> {
        self.cx.update_window(self.window.into(), |_, window, _| window.refresh()).map_err(|e| e.to_string())?;
        self.cx.run_until_parked();
        self.cx.capture_screenshot(self.window.into()).map_err(|e| e.to_string())
    }

    /// The window once it stops changing (folders, Git scans and thumbnails arrive late).
    fn settled_image(&mut self) -> Result<RgbaImage, String> {
        let mut last = self.image()?;
        for _ in 0..40 {
            self.pump(250);
            let next = self.image()?;
            if next.as_raw() == last.as_raw() {
                return Ok(next);
            }
            last = next;
        }
        Err("the window never stopped changing".into())
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    // io-ok: developer tool
    io(std::fs::create_dir_all(to))?;
    // io-ok: developer tool
    for entry in io(std::fs::read_dir(from))? {
        let entry = io(entry)?;
        let target = to.join(entry.file_name());
        // io-ok: developer tool
        if io(entry.file_type())?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            // io-ok: developer tool
            io(std::fs::copy(entry.path(), target))?;
        }
    }
    Ok(())
}

/// Render `name` in `theme`: the component at 2x, padded.
pub fn render(name: &str, theme: &str) -> Result<RgbaImage, String> {
    render_with(name, theme, false)
}

/// `render`, or with `whole_window` the full window the component is cut from (to see why a selector is missing).
pub fn render_with(name: &str, theme: &str, whole_window: bool) -> Result<RgbaImage, String> {
    let scratch = Scratch::new()?;
    let plan = plan(name, &scratch)?;
    let mut session = Session::open(theme, &plan, &scratch)?;
    session.keys(plan.keys)?;
    if plan.git_diff {
        session.show_git_diff()?;
    }
    let window_image = session.settled_image()?;
    if whole_window {
        return Ok(window_image);
    }
    let mut union: Option<Bounds<Pixels>> = None;
    let mut add = |bounds: Bounds<Pixels>| union = Some(union.map_or(bounds, |u| u.union(&bounds)));
    for selector in &plan.crop.selectors {
        add(session
            .bounds(selector)
            .ok_or_else(|| format!("\"{selector}\" is not on screen (is this a debug build?)"))?);
    }
    for index in 0..plan.crop.numbered.map_or(0, |_| 64) {
        if let Some(bounds) = plan.crop.numbered.and_then(|prefix| session.bounds(&format!("{prefix}{index}"))) {
            add(bounds);
        }
    }
    let bounds = union.ok_or("nothing to crop: no element with the wanted selector (is this a debug build?)")?;
    Ok(cut(&window_image, bounds, plan.grow))
}

/// `bounds` (points) cut out of `window` and padded with the colour at the crop's corner.
fn cut(window: &RgbaImage, bounds: Bounds<Pixels>, grow: [f32; 4]) -> RgbaImage {
    let px = |v: f32| (v * SCALE).round() as i64;
    let [top, right, bottom, left] = grow;
    let x0 = (px(f32::from(bounds.left())) - px(left)).clamp(0, window.width() as i64) as u32;
    let y0 = (px(f32::from(bounds.top())) - px(top)).clamp(0, window.height() as i64) as u32;
    let x1 = (px(f32::from(bounds.right())) + px(right)).clamp(0, window.width() as i64) as u32;
    let y1 = (px(f32::from(bounds.bottom())) + px(bottom)).clamp(0, window.height() as i64) as u32;
    let (w, h) = (x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1));
    let pad = px(PAD) as u32;
    let fill = *window.get_pixel(x0.min(window.width() - 1), y0.min(window.height() - 1));
    let mut out = RgbaImage::from_pixel(w + 2 * pad, h + 2 * pad, fill);
    for y in 0..h {
        for x in 0..w {
            out.put_pixel(
                x + pad,
                y + pad,
                *window.get_pixel((x0 + x).min(window.width() - 1), (y0 + y).min(window.height() - 1)),
            );
        }
    }
    out
}

pub fn run(args: &[String]) -> ExitCode {
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let (Some(name), Some(out)) = (value("--component-shot"), value("--out")) else {
        eprintln!(
            "usage: crow --component-shot NAME [--theme ID] [--scale 2] --out FILE.png\nNAME: {}",
            COMPONENTS.join(", ")
        );
        return ExitCode::FAILURE;
    };
    if value("--scale").is_some_and(|scale| scale != "2") {
        eprintln!("crow --component-shot: only --scale 2 (GPUI's headless window is fixed at 2x)");
        return ExitCode::FAILURE;
    }
    let theme = value("--theme").unwrap_or_else(|| "finder-light".to_owned());
    match render_with(&name, &theme, args.iter().any(|a| a == "--whole-window"))
        .and_then(|image| image.save(&out).map_err(|e| e.to_string()))
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("crow --component-shot: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (component, minimum width, minimum height) of its 2x image, padding included.
    const SIZES: [(&str, u32, u32); 5] = [
        ("git-rows", 600, 200),
        ("git-sidebar", 400, 250),
        ("git-diff", 400, 500),
        ("compare-gallery", 900, 400),
        ("plugin-surfaces", 900, 350),
    ];

    fn distinct_colours(image: &RgbaImage) -> usize {
        let mut seen = std::collections::HashSet::new();
        for pixel in image.pixels() {
            seen.insert(pixel.0);
            if seen.len() > 64 {
                break;
            }
        }
        seen.len()
    }

    #[test]
    fn every_component_renders_a_padded_non_blank_image() {
        assert_eq!(SIZES.map(|(name, ..)| name), COMPONENTS);
        for (name, min_width, min_height) in SIZES {
            for theme in ["finder-light", "finder-dark"] {
                let image = render(name, theme).unwrap_or_else(|e| panic!("{name} in {theme}: {e}"));
                assert!(
                    image.width() >= min_width && image.height() >= min_height,
                    "{name} in {theme} is {}x{}, expected at least {min_width}x{min_height}",
                    image.width(),
                    image.height()
                );
                assert!(distinct_colours(&image) > 16, "{name} in {theme} looks blank");
                let pad = (PAD * SCALE) as u32;
                let corner = *image.get_pixel(0, 0);
                assert!(
                    (0..pad).all(|i| *image.get_pixel(i, i) == corner),
                    "{name} in {theme}: the padding is one colour"
                );
            }
        }
    }

    #[test]
    fn the_light_and_dark_shots_differ_and_a_render_is_repeatable() {
        let light = render("git-rows", "finder-light").unwrap();
        let dark = render("git-rows", "finder-dark").unwrap();
        assert_eq!((light.width(), light.height()), (dark.width(), dark.height()));
        assert_ne!(light.as_raw(), dark.as_raw());
        assert_eq!(light.as_raw(), render("git-rows", "finder-light").unwrap().as_raw(), "deterministic");
    }

    #[test]
    fn an_unknown_component_is_refused() {
        assert!(render("nope", "finder-light").unwrap_err().contains("unknown component"));
    }
}
