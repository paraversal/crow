//! Embedded Lucide stroke icons, tinted with a theme role.
//!
//! Icons are Lucide 1.72.0 (https://lucide.dev), unmodified. Lucide is
//! ISC-licensed; the notice is `assets/icons/LICENSE` and ships in the app
//! bundle's Resources.

use std::borrow::Cow;

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
            Glyph::GoTo => "icons/arrow-right.svg",
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
        }
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
    ("icons/arrow-right.svg", include_bytes!("../assets/icons/arrow-right.svg")),
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
pub fn pack_glyph(asset: &[u8], size: f32, tint: Rgba) -> Svg {
    svg().data(asset).w(px(size)).h(px(size)).flex_none().text_color(tint).debug_selector(|| "pack-glyph".into())
}

#[cfg(test)]
mod tests {
    use super::*;

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
