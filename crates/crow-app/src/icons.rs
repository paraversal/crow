//! Embedded Lucide stroke icons, tinted with a theme role.
//!
//! Icons are Lucide 1.72.0 (https://lucide.dev), unmodified. Lucide is
//! ISC-licensed; the notice is `assets/icons/LICENSE` and ships in the app
//! bundle's Resources.

use std::borrow::Cow;
use std::path::Path;

use crow_theme::Theme;
use gpui::{AssetSource, Result, Rgba, SharedString, Svg, prelude::*, px, svg};

use crate::style::color;

// U3–U7 draw the other glyphs; U1 uses Back, Forward, List and Columns.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Back,
    Forward,
    Grid,
    List,
    Columns,
    Gallery,
    Search,
    GoTo,
    Close,
    ChevronDown,
    ChevronRight,
    AlertCircle,
    Trash,
    Pencil,
    Folder,
    Tag,
    Clock,
    Cloud,
    CloudOff,
    Lock,
    Home,
    Desktop,
    Documents,
    Downloads,
    Applications,
    Movies,
    Music,
    Pictures,
    Public,
    Library,
    Developer,
    /// The startup disk in Locations.
    Drive,
    /// Any other volume in Locations.
    ExternalDrive,
}

impl Glyph {
    #[allow(dead_code)]
    pub const ALL: &'static [Glyph] = &[
        Glyph::Back,
        Glyph::Forward,
        Glyph::Grid,
        Glyph::List,
        Glyph::Columns,
        Glyph::Gallery,
        Glyph::Search,
        Glyph::GoTo,
        Glyph::Close,
        Glyph::ChevronDown,
        Glyph::ChevronRight,
        Glyph::AlertCircle,
        Glyph::Trash,
        Glyph::Pencil,
        Glyph::Folder,
        Glyph::Tag,
        Glyph::Clock,
        Glyph::Cloud,
        Glyph::CloudOff,
        Glyph::Lock,
        Glyph::Home,
        Glyph::Desktop,
        Glyph::Documents,
        Glyph::Downloads,
        Glyph::Applications,
        Glyph::Movies,
        Glyph::Music,
        Glyph::Pictures,
        Glyph::Public,
        Glyph::Library,
        Glyph::Developer,
        Glyph::Drive,
        Glyph::ExternalDrive,
    ];

    pub fn path(self) -> &'static str {
        match self {
            Glyph::Back => "icons/chevron-left.svg",
            Glyph::Forward => "icons/chevron-right.svg",
            Glyph::Grid => "icons/layout-grid.svg",
            Glyph::List => "icons/list.svg",
            Glyph::Columns => "icons/columns-3.svg",
            Glyph::Gallery => "icons/gallery-vertical.svg",
            Glyph::Search => "icons/search.svg",
            Glyph::GoTo => "icons/arrow-right-to-line.svg",
            Glyph::Close => "icons/x.svg",
            Glyph::ChevronDown => "icons/chevron-down.svg",
            Glyph::ChevronRight => "icons/chevron-right.svg",
            Glyph::AlertCircle => "icons/alert-circle.svg",
            Glyph::Trash => "icons/trash-2.svg",
            Glyph::Pencil => "icons/pencil.svg",
            Glyph::Folder => "icons/folder.svg",
            Glyph::Tag => "icons/tag.svg",
            Glyph::Clock => "icons/clock.svg",
            Glyph::Cloud => "icons/cloud.svg",
            Glyph::CloudOff => "icons/cloud-off.svg",
            Glyph::Lock => "icons/lock.svg",
            Glyph::Home => "icons/house.svg",
            Glyph::Desktop => "icons/monitor.svg",
            Glyph::Documents => "icons/file-text.svg",
            Glyph::Downloads => "icons/circle-arrow-down.svg",
            Glyph::Applications => "icons/app-window.svg",
            Glyph::Movies => "icons/clapperboard.svg",
            Glyph::Music => "icons/music.svg",
            Glyph::Pictures => "icons/image.svg",
            Glyph::Public => "icons/users.svg",
            Glyph::Library => "icons/library.svg",
            Glyph::Developer => "icons/hammer.svg",
            Glyph::Drive | Glyph::ExternalDrive => "icons/hard-drive.svg",
        }
    }
}

/// The line glyph Finder's sidebar draws for a special folder (house for Home, a monitor for
/// Desktop…), so a native sidebar looks the way Finder's does. `None` for any other folder.
pub fn special_folder_glyph(path: &Path, home: &Path) -> Option<Glyph> {
    if path == home {
        return Some(Glyph::Home);
    }
    if path == Path::new("/Applications") {
        return Some(Glyph::Applications);
    }
    if path == Path::new("/Library") {
        return Some(Glyph::Library);
    }
    if path.parent() != Some(home) {
        return None;
    }
    Some(match path.file_name()?.to_str()? {
        "Desktop" => Glyph::Desktop,
        "Documents" => Glyph::Documents,
        "Downloads" => Glyph::Downloads,
        "Applications" => Glyph::Applications,
        "Movies" => Glyph::Movies,
        "Music" => Glyph::Music,
        "Pictures" => Glyph::Pictures,
        "Public" => Glyph::Public,
        "Library" => Glyph::Library,
        "Developer" => Glyph::Developer,
        _ => return None,
    })
}

/// The outline glyph for any folder: its special glyph, a drive for the startup disk or a
/// mounted volume, else a plain folder.
pub fn outline_folder_glyph(path: &Path, home: &Path) -> Glyph {
    special_folder_glyph(path, home).unwrap_or_else(|| {
        if path == Path::new("/") {
            Glyph::Drive
        } else if path.parent() == Some(Path::new("/Volumes")) {
            Glyph::ExternalDrive
        } else {
            Glyph::Folder
        }
    })
}

/// The SF Symbol Finder's sidebar draws for `glyph`, for the glyphs that have one.
fn symbol_name(glyph: Glyph) -> Option<&'static str> {
    Some(match glyph {
        Glyph::Folder => "folder",
        Glyph::Home => "house",
        Glyph::Desktop => "dock.rectangle",
        Glyph::Documents => "doc",
        Glyph::Downloads => "arrow.down.circle",
        Glyph::Applications => "a.square",
        Glyph::Movies => "film",
        Glyph::Music => "music.note",
        Glyph::Pictures => "photo.fill",
        Glyph::Public => "figure.walk.diamond",
        Glyph::Library => "building.columns",
        Glyph::Developer => "hammer",
        Glyph::Drive => "internaldrive",
        Glyph::ExternalDrive => "externaldrive",
        _ => return None,
    })
}

/// An outline sidebar icon: the system's SF Symbol in `tint`, so it follows the macOS version
/// like Finder's sidebar; the shipped Lucide glyph when the symbol is missing or off the main
/// thread.
pub fn outline_glyph(name: Glyph, size: f32, tint: Rgba) -> gpui::AnyElement {
    let rgba = [tint.r, tint.g, tint.b, tint.a].map(|channel| (channel.clamp(0., 1.) * 255.0).round() as u8);
    let image = symbol_name(name)
        .and_then(|symbol| crow_macos::symbol_png(symbol, size.round() as u32, rgba))
        .and_then(|(png, width, height)| Some((crate::style::image_from_png(png)?, width, height)));
    match image {
        // Drawn at the symbol's own size and proportions, in points.
        Some((image, width, height)) => gpui::img(image).w(px(width)).h(px(height)).flex_none().into_any_element(),
        None => glyph_color(name, size, tint).into_any_element(),
    }
}

pub struct Assets;

const FILES: &[(&str, &[u8])] = &[
    ("icons/chevron-left.svg", include_bytes!("../assets/icons/chevron-left.svg")),
    ("icons/chevron-right.svg", include_bytes!("../assets/icons/chevron-right.svg")),
    ("icons/layout-grid.svg", include_bytes!("../assets/icons/layout-grid.svg")),
    ("icons/list.svg", include_bytes!("../assets/icons/list.svg")),
    ("icons/columns-3.svg", include_bytes!("../assets/icons/columns-3.svg")),
    ("icons/gallery-vertical.svg", include_bytes!("../assets/icons/gallery-vertical.svg")),
    ("icons/search.svg", include_bytes!("../assets/icons/search.svg")),
    ("icons/arrow-right-to-line.svg", include_bytes!("../assets/icons/arrow-right-to-line.svg")),
    ("icons/x.svg", include_bytes!("../assets/icons/x.svg")),
    ("icons/chevron-down.svg", include_bytes!("../assets/icons/chevron-down.svg")),
    ("icons/alert-circle.svg", include_bytes!("../assets/icons/alert-circle.svg")),
    ("icons/trash-2.svg", include_bytes!("../assets/icons/trash-2.svg")),
    ("icons/pencil.svg", include_bytes!("../assets/icons/pencil.svg")),
    ("icons/folder.svg", include_bytes!("../assets/icons/folder.svg")),
    ("icons/tag.svg", include_bytes!("../assets/icons/tag.svg")),
    ("icons/clock.svg", include_bytes!("../assets/icons/clock.svg")),
    ("icons/cloud.svg", include_bytes!("../assets/icons/cloud.svg")),
    ("icons/cloud-off.svg", include_bytes!("../assets/icons/cloud-off.svg")),
    ("icons/lock.svg", include_bytes!("../assets/icons/lock.svg")),
    ("icons/house.svg", include_bytes!("../assets/icons/house.svg")),
    ("icons/monitor.svg", include_bytes!("../assets/icons/monitor.svg")),
    ("icons/file-text.svg", include_bytes!("../assets/icons/file-text.svg")),
    ("icons/circle-arrow-down.svg", include_bytes!("../assets/icons/circle-arrow-down.svg")),
    ("icons/app-window.svg", include_bytes!("../assets/icons/app-window.svg")),
    ("icons/clapperboard.svg", include_bytes!("../assets/icons/clapperboard.svg")),
    ("icons/music.svg", include_bytes!("../assets/icons/music.svg")),
    ("icons/image.svg", include_bytes!("../assets/icons/image.svg")),
    ("icons/users.svg", include_bytes!("../assets/icons/users.svg")),
    ("icons/library.svg", include_bytes!("../assets/icons/library.svg")),
    ("icons/hammer.svg", include_bytes!("../assets/icons/hammer.svg")),
    ("icons/hard-drive.svg", include_bytes!("../assets/icons/hard-drive.svg")),
    ("icons/LICENSE", include_bytes!("../assets/icons/LICENSE")),
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(FILES.iter().find(|(name, _)| *name == path).map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(FILES.iter().filter(|(name, _)| name.starts_with(path)).map(|(name, _)| SharedString::from(*name)).collect())
    }
}

/// One icon at `size` points, tinted with `role`.
pub fn glyph(name: Glyph, size: f32, role: &str, theme: &Theme) -> Svg {
    glyph_color(name, size, color(theme.role(role)))
}

/// One icon in an explicit color, for a row whose own color is a palette slot
/// rather than a theme role (a sidebar favorite's `color`).
pub fn glyph_color(name: Glyph, size: f32, tint: Rgba) -> Svg {
    svg().path(name.path()).w(px(size)).h(px(size)).flex_none().text_color(tint)
}

/// One packed glyph from already-validated, in-memory bytes. GPUI hashes the
/// bytes for its cache key and paints from memory, so a theme change reuses the
/// alpha mask and no file is read at render time.
pub fn pack_glyph(icon: &crate::icon_packs::ResolvedIcon, size: f32, tint: Rgba) -> gpui::AnyElement {
    match &icon.image {
        // A PNG glyph is a full-color picture: no tint, scaled to the icon box.
        Some(image) => gpui::img(image.clone())
            .w(px(size))
            .h(px(size))
            .flex_none()
            .debug_selector(|| "pack-glyph".into())
            .into_any_element(),
        None => svg()
            .data(&icon.asset)
            .w(px(size))
            .h(px(size))
            .flex_none()
            .text_color(tint)
            .debug_selector(|| "pack-glyph".into())
            .into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_folders_get_finders_sidebar_glyphs() {
        let home = Path::new("/Users/me");
        assert_eq!(special_folder_glyph(home, home), Some(Glyph::Home));
        assert_eq!(special_folder_glyph(Path::new("/Users/me/Downloads"), home), Some(Glyph::Downloads));
        assert_eq!(special_folder_glyph(Path::new("/Applications"), home), Some(Glyph::Applications));
        assert_eq!(special_folder_glyph(Path::new("/Users/me/Projects"), home), None);
        assert_eq!(special_folder_glyph(Path::new("/Users/me/Projects/Downloads"), home), None);
    }

    #[test]
    fn every_glyph_resolves() {
        for glyph in Glyph::ALL {
            let bytes = Assets
                .load(glyph.path())
                .expect("the asset source is infallible")
                .unwrap_or_else(|| panic!("{} is embedded", glyph.path()));
            let text = std::str::from_utf8(&bytes).unwrap_or_else(|_| panic!("{} is UTF-8", glyph.path()));
            assert!(text.contains("<svg"), "{} is an SVG", glyph.path());
            assert!(text.contains("viewBox=\"0 0 24 24\""), "{} uses the 24 px grid", glyph.path());
        }
    }

    #[test]
    fn assets_list_icons() {
        let listed = Assets.list("icons/").expect("the asset source is infallible");
        for glyph in Glyph::ALL {
            assert!(listed.iter().any(|name| name == glyph.path()), "{} is listed", glyph.path());
        }
        assert!(Assets.load("icons/missing.svg").expect("infallible").is_none());
    }
}
