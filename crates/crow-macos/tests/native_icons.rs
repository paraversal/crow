//! AppKit must encode the icons on the process's main thread.
use std::os::unix::fs::symlink;
use std::sync::Arc;

use crow_fs::{IconKind, stat_entry};
use crow_macos::{ComboDrag, begin_drag, icon_png, symbol_png, type_icon_png};
use raw_window_handle::{HandleError, HasWindowHandle, WindowHandle};

fn main() {
    struct UnneededWindow;
    impl HasWindowHandle for UnneededWindow {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            panic!("an empty drag must not ask for a window handle");
        }
    }

    let empty_drag = ComboDrag {
        paths: &[],
        icon_png: &[],
        label: "",
        width: 22.,
        height: 22.,
        pill_width: 0.,
        cursor_from_left: 0.,
        cursor_from_top: 0.,
    };
    assert!(!begin_drag(&UnneededWindow, &empty_drag));

    let root = tempfile::tempdir().expect("temporary icon fixtures");
    for (alias, plain) in [("link", ".localized"), ("a.txt", "b.txt")] {
        let plain = root.path().join(plain);
        let link = root.path().join(alias);
        std::fs::write(&plain, "contents").unwrap();
        symlink(&plain, &link).unwrap();

        let link = stat_entry(&link).unwrap().unwrap();
        let plain = stat_entry(&plain).unwrap().unwrap();
        assert_eq!(link.icon_kind, IconKind::Path);
        assert_eq!(plain.icon_kind, IconKind::File);
        let link_png = icon_png(&link.path, link.icon_kind, 32).expect("AppKit symlink icon");
        let plain_png = icon_png(&plain.path, plain.icon_kind, 32).expect("AppKit plain file icon");
        assert_ne!(link_png, plain_png, "a symlink must never seed the plain file's cache");
        let type_png = type_icon_png(plain.ext(), false, 32).expect("AppKit file type icon");
        assert!(Arc::ptr_eq(&plain_png, &type_png), "plain files reuse type icons");
    }
    // A file named example.dir and a folder have independent type cache keys.
    let folder = type_icon_png("", true, 32).expect("AppKit folder icon");
    let file = type_icon_png("dir", false, 32).expect("AppKit file icon");
    assert!(!Arc::ptr_eq(&file, &folder));
    // SF Symbols: Finder's sidebar glyphs, in one color; a missing name is None.
    let (house, width, height) = symbol_png("house", 16, [10, 132, 255, 255]).expect("the house symbol");
    let (red, ..) = symbol_png("house", 16, [255, 0, 0, 255]).expect("the house symbol, red");
    assert_ne!(house, red, "the color is part of the icon");
    assert!(width > height && (14.0..=22.0).contains(&width), "drawn at its natural size in points: {width}x{height}");
    let (_, doc_width, doc_height) = symbol_png("doc", 16, [10, 132, 255, 255]).expect("the doc symbol");
    // The glyph's proportions depend on the macOS version (the CI runner draws "doc" square).
    assert!(doc_width <= doc_height, "a document is not wider than tall: {doc_width}x{doc_height}");
    assert!(symbol_png("no.such.symbol.name", 16, [0, 0, 0, 255]).is_none());
    if let Some(dir) = std::env::var_os("CROW_SYMBOL_DUMP") {
        for name in ["house", "dock.rectangle", "doc", "arrow.down.circle", "folder", "a.square"] {
            let (png, ..) = symbol_png(name, 32, [10, 132, 255, 255]).unwrap();
            std::fs::write(std::path::Path::new(&dir).join(format!("{name}.png")), png.as_slice()).unwrap();
        }
    }
}
