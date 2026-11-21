//! AppKit must encode the icons on the process's main thread.
use std::os::unix::fs::symlink;
use std::sync::Arc;

use crow_fs::{IconKind, stat_entry};
use crow_macos::{icon_png, type_icon_png};

fn main() {
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
}
