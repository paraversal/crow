//! Embedded Lucide stroke icons, tinted with a theme role.
//!
//! Icons are Lucide 1.72.0 (https://lucide.dev), unmodified. Lucide is
//! ISC-licensed; the notice is `assets/icons/LICENSE` and ships in the app
//! bundle's Resources.

use std::borrow::Cow;

use crow_theme::Theme;
use gpui::{AssetSource, Result, SharedString, Svg, prelude::*, px, svg};

use crate::style::color;

// U3–U7 draw the other glyphs; U1 uses Back, Forward, List and Columns.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Back,
    Forward,
    List,
    Columns,
    Search,
    GoTo,
    Close,
    ChevronDown,
    ChevronRight,
    AlertCircle,
}

impl Glyph {
    #[allow(dead_code)]
    pub const ALL: &'static [Glyph] = &[
        Glyph::Back,
        Glyph::Forward,
        Glyph::List,
        Glyph::Columns,
        Glyph::Search,
        Glyph::GoTo,
        Glyph::Close,
        Glyph::ChevronDown,
        Glyph::ChevronRight,
        Glyph::AlertCircle,
    ];

    pub fn path(self) -> &'static str {
        match self {
            Glyph::Back => "icons/chevron-left.svg",
            Glyph::Forward => "icons/chevron-right.svg",
            Glyph::List => "icons/list.svg",
            Glyph::Columns => "icons/columns-3.svg",
            Glyph::Search => "icons/search.svg",
            Glyph::GoTo => "icons/arrow-right.svg",
            Glyph::Close => "icons/x.svg",
            Glyph::ChevronDown => "icons/chevron-down.svg",
            Glyph::ChevronRight => "icons/chevron-right.svg",
            Glyph::AlertCircle => "icons/alert-circle.svg",
        }
    }
}

pub struct Assets;

const FILES: &[(&str, &[u8])] = &[
    ("icons/chevron-left.svg", include_bytes!("../assets/icons/chevron-left.svg")),
    ("icons/chevron-right.svg", include_bytes!("../assets/icons/chevron-right.svg")),
    ("icons/list.svg", include_bytes!("../assets/icons/list.svg")),
    ("icons/columns-3.svg", include_bytes!("../assets/icons/columns-3.svg")),
    ("icons/search.svg", include_bytes!("../assets/icons/search.svg")),
    ("icons/arrow-right.svg", include_bytes!("../assets/icons/arrow-right.svg")),
    ("icons/x.svg", include_bytes!("../assets/icons/x.svg")),
    ("icons/chevron-down.svg", include_bytes!("../assets/icons/chevron-down.svg")),
    ("icons/alert-circle.svg", include_bytes!("../assets/icons/alert-circle.svg")),
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
    svg().path(name.path()).w(px(size)).h(px(size)).flex_none().text_color(color(theme.role(role)))
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
