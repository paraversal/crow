use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{Action, Entity, TestAppContext, VisualTestContext, WindowHandle};

use crow_fs::testing::{Churn, ChurnHandle, EdgeTree};
use crow_search::{Hit, HitKind};

use crate::actions::StartFilter;

use super::*;

fn test_ranges(start: usize, end: usize) -> Vec<std::ops::Range<usize>> {
    std::iter::once(start..end).collect()
}

/// `root/{alpha/inner/, beta/, notes.txt}`, canonicalized (macOS temp dirs sit behind /private).
///
/// The base is `/tmp`, not `std::env::temp_dir()`: the palette scores a folder's
/// whole path, so a random `$TMPDIR` prefix can make a query match an unrelated
/// sibling. `palette_count_says_places` flaked on CI when the runner's temp path
/// happened to contain `a-l-p-h-a` in order and `beta` matched `alpha`.
fn fixture(name: &str) -> PathBuf {
    let root = PathBuf::from("/tmp").join(format!("crow-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("alpha/inner")).expect("create fixture");
    std::fs::create_dir_all(root.join("beta")).expect("create fixture");
    std::fs::write(root.join("notes.txt"), "hello\n").expect("create fixture");
    root.canonicalize().expect("canonical fixture path")
}

#[gpui::test]
fn first_row_sits_below_the_header_line(cx: &mut TestAppContext) {
    let root = fixture("list-inset");
    let (_view, mut cx) = open(cx, &root);
    let header = cx.debug_bounds("table-header").expect("header");
    let row = cx.debug_bounds("row-alpha").expect("first row");
    assert_eq!(row.top() - header.bottom(), px(crate::pane::LIST_TOP_INSET));
}

#[gpui::test]
fn filter_count_is_announced(cx: &mut TestAppContext) {
    let root = fixture("filter-announce");
    let executor = cx.executor();
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("/");
    cx.simulate_input("a");
    cx.simulate_input("l");
    let announced =
        |cx: &VisualTestContext| view.read_with(cx, |w, cx| w.pane().read(cx).filter_announcement().to_string());
    assert_eq!(announced(&cx), "3 of 3 shown", "typing does not queue one announcement per letter");
    executor.advance_clock(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    assert_eq!(announced(&cx), "1 of 3 shown", "the count is announced once typing stops");
}

#[gpui::test]
fn search_count_is_announced(cx: &mut TestAppContext) {
    let root = fixture("search-announce");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("notes");
    wait_for_search(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    assert_eq!(search.read_with(&cx, |search, _| search.footer_summary_text()), "1 result");
}

#[gpui::test]
fn assembled_layout_matches_boards(cx: &mut TestAppContext) {
    let root = fixture("assembled");
    let (view, mut cx) = open_sized(cx, &root, 1280., 800.);
    pane(&view, &cx).update(&mut cx, |pane, cx| pane.set_mode(ViewMode::Columns, cx));
    settle(&view, &mut cx);
    assert_eq!(cx.debug_bounds("toolbar").expect("band").size.height, px(52.), "band");
    assert_eq!(cx.debug_bounds("sidebar").expect("sidebar").size.width, px(210.), "sidebar");
    assert_eq!(cx.debug_bounds("path-bar").expect("bottom bar").size.height, px(30.), "bottom bar");
    assert_eq!(cx.debug_bounds("column-0-row-alpha").expect("a column row").size.height, px(24.), "row");
    cx.simulate_keystrokes("/");
    cx.simulate_input("vi");
    assert_eq!(cx.debug_bounds("filter-field").expect("filter field").size.height, px(28.), "filter field");
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("cmd-f");
    wait_for_search(&view, &mut cx);
    assert_eq!(cx.debug_bounds("search-scope-bar").expect("scope bar").size.height, px(38.), "scope bar");
}

#[gpui::test]
fn same_key_everywhere(cx: &mut TestAppContext) {
    let root = fixture("same-key");
    let (view, mut cx) = open(cx, &root);
    let actions: [(&str, &dyn gpui::Action); 5] = [
        ("ShowSearch", &ShowSearch),
        ("ShowCommandPalette", &ShowCommandPalette),
        ("StartFilter", &StartFilter),
        ("GoToFolder", &GoToFolder),
        ("SearchReveal", &crate::actions::SearchReveal),
    ];
    for vim in [false, true] {
        if vim {
            use_vim(&view, &mut cx);
        }
        for (label, action) in actions {
            let mut seen: Vec<(KeyScope, String)> = Vec::new();
            for scope in [
                KeyScope::Window,
                KeyScope::List,
                KeyScope::Columns,
                KeyScope::Sidebar,
                KeyScope::Filter,
                KeyScope::Search,
                KeyScope::SearchResults,
                KeyScope::Palette,
            ] {
                if let Some(text) = cx.read(|cx| keys::text(action, scope, cx).map(|text| text.to_string())) {
                    seen.push((scope, text));
                }
            }
            assert!(!seen.is_empty(), "{label} has a key");
            let first = seen[0].1.clone();
            for (scope, text) in &seen {
                assert_eq!(*text, first, "{label} shows one key everywhere ({scope:?}, vim: {vim})");
            }
        }
    }
}

#[gpui::test]
fn long_names_stay_in_their_rows(cx: &mut TestAppContext) {
    let root = fixture("long-names");
    for name in [
        format!("{}.txt", "a".repeat(200)),
        "🙂 ملف عربي.txt".to_owned(),
        "Résumé.txt".to_owned(),
        // NFD only (no NFC twin): APFS is normalization-insensitive, so a twin
        // would be the same entry; this name keeps its combining marks.
        "Cafe\u{301}.txt".to_owned(),
    ] {
        std::fs::write(root.join(name), "x").expect("create fixture");
    }
    let mut deep = root.clone();
    for level in 0..20 {
        deep = deep.join(format!("level-{level}"));
    }
    std::fs::create_dir_all(&deep).expect("create fixture");
    // The names as the listing reports them (the filesystem may normalize).
    let names: Vec<String> = std::fs::read_dir(&root)
        .expect("read fixture")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !matches!(name.as_str(), "alpha" | "beta" | "notes.txt"))
        .collect();
    assert!(names.len() >= 4, "the edge-case names exist: {names:?}");
    for width in [1280., 700.] {
        let (view, mut cx) = open_sized(cx, &root, width, 800.);
        let list = cx.debug_bounds("file-list").expect("list");
        let selector = |text: String| Box::leak(text.into_boxed_str()) as &'static str;
        for name in &names {
            let row = cx.debug_bounds(selector(format!("row-{name}"))).expect("list row");
            assert!(
                row.left() >= list.left() && row.right() <= list.right(),
                "{name:?} stays inside its row at {width}"
            );
            let text = cx.debug_bounds(selector(format!("row-name-{name}"))).expect("list name");
            assert!(
                text.left() >= row.left() && text.right() <= row.right(),
                "{name:?} text stays inside its row at {width}"
            );
        }
        pane(&view, &cx).update(&mut cx, |pane, cx| pane.set_mode(ViewMode::Columns, cx));
        settle(&view, &mut cx);
        let strip = cx.debug_bounds("columns").expect("strip");
        for name in &names {
            let row = cx.debug_bounds(selector(format!("column-0-row-{name}"))).expect("column row");
            assert!(
                row.left() >= strip.left() && row.right() <= strip.right(),
                "{name:?} stays inside its column at {width}"
            );
            let text = cx.debug_bounds(selector(format!("column-0-row-name-{name}"))).expect("column name");
            assert!(
                text.left() >= row.left() && text.right() <= row.right(),
                "{name:?} text stays inside its column at {width}"
            );
        }
        // The deep folder as a favorite and in the path bar.
        write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&deep]));
        let sidebar = cx.debug_bounds("sidebar").expect("sidebar");
        let favorite = cx.debug_bounds("sidebar-0").expect("favorite row");
        assert!(favorite.right() <= sidebar.right(), "the favorite stays inside the sidebar at {width}");
        pane(&view, &cx).update(&mut cx, |pane, cx| pane.navigate(deep.clone(), cx));
        settle(&view, &mut cx);
        let bar = cx.debug_bounds("path-bar").expect("bar");
        for i in 0..12 {
            let Some(segment) = cx.debug_bounds(selector(format!("path-segment-{i}"))) else { break };
            assert!(segment.right() <= bar.right(), "path segment {i} stays inside the bar at {width}");
        }
    }
}

#[gpui::test]
fn one_accent_fill_in_column_view(cx: &mut TestAppContext) {
    let root = fixture("one-fill");
    std::fs::create_dir_all(root.join("alpha/inner")).expect("create fixture");
    let (view, mut cx) = open_sized(cx, &root, 1280., 800.);
    pane(&view, &cx).update(&mut cx, |pane, cx| pane.set_mode(ViewMode::Columns, cx));
    settle(&view, &mut cx);
    let fills =
        |cx: &mut VisualTestContext| cx.update(|window, cx| view.read(cx).pane().read(cx).selection_fills(window, cx));
    assert_eq!(fills(&mut cx), (1, 0), "the pane owns the keys: one accent fill");
    cx.simulate_keystrokes("tab");
    assert_eq!(fills(&mut cx), (0, 1), "with the sidebar focused the pane's fill is inactive");
}

#[gpui::test]
fn column_preview_fills_the_rest(cx: &mut TestAppContext) {
    let root = fixture("preview-fills");
    let (view, mut cx) = open_sized(cx, &root, 1280., 800.);
    pane(&view, &cx).update(&mut cx, |pane, cx| pane.set_mode(ViewMode::Columns, cx));
    settle(&view, &mut cx);
    // alpha and beta are folders; notes.txt is a file, so no child column follows it.
    cx.simulate_keystrokes("down down");
    settle(&view, &mut cx);
    let strip = cx.debug_bounds("columns").expect("strip");
    let column = cx.debug_bounds("column-0").expect("first column");
    let preview = cx.debug_bounds("preview-column").expect("preview column");
    assert_eq!(preview.size.width, strip.size.width - column.size.width, "the preview takes the rest");
}

#[gpui::test]
fn no_matches_show_the_folder_summary(cx: &mut TestAppContext) {
    let root = fixture("no-match-summary");
    let src = root.join("src");
    std::fs::create_dir_all(&src).expect("create fixture");
    std::fs::write(src.join("a.txt"), "x").expect("create fixture");
    let (view, mut cx) = open_sized(cx, &src, 1280., 800.);
    pane(&view, &cx).update(&mut cx, |pane, cx| pane.set_mode(ViewMode::Columns, cx));
    settle(&view, &mut cx);
    cx.simulate_keystrokes("/");
    cx.simulate_input("zzz");
    settle(&view, &mut cx);
    let label = view.read_with(&cx, |w, cx| w.pane().read(cx).preview_label(cx));
    assert_eq!(label.as_deref(), Some("src, Folder, 1 item"), "the strip is never an empty quarter");
}

#[gpui::test]
fn unreadable_folder_summary_has_no_count(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let root = fixture("locked-summary");
    let locked = root.join("locked");
    std::fs::create_dir_all(&locked).expect("create fixture");
    std::fs::write(locked.join("a.txt"), "x").expect("create fixture");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("lock fixture");
    let (view, mut cx) = open_sized(cx, &locked, 1280., 800.);
    pane(&view, &cx).update(&mut cx, |pane, cx| pane.set_mode(ViewMode::Columns, cx));
    settle(&view, &mut cx);
    let label = view.read_with(&cx, |w, cx| w.pane().read(cx).preview_label(cx));
    // The same folder under the cursor, with its child column failing to list.
    pane(&view, &cx).update(&mut cx, |pane, cx| pane.reveal(locked.clone(), cx));
    settle(&view, &mut cx);
    let cursor_label = view.read_with(&cx, |w, cx| w.pane().read(cx).preview_label(cx));
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).expect("unlock fixture");
    assert_eq!(label.as_deref(), Some("locked, Folder"), "a folder that can't be listed has no item count");
    assert_eq!(cursor_label.as_deref(), Some("locked, Folder"), "nor when the cursor is on it");
}

#[gpui::test]
fn folder_cursor_shows_summary(cx: &mut TestAppContext) {
    let root = fixture("folder-summary");
    let (view, mut cx) = open_sized(cx, &root, 1280., 800.);
    pane(&view, &cx).update(&mut cx, |pane, cx| pane.set_mode(ViewMode::Columns, cx));
    settle(&view, &mut cx);
    let label = view.read_with(&cx, |window, cx| window.pane().read(cx).preview_label(cx));
    assert_eq!(label.as_deref(), Some("alpha, Folder, 1 item"));
}

fn open(cx: &mut TestAppContext, dir: &Path) -> (Entity<CrowWindow>, VisualTestContext) {
    open_with(cx, dir, Default::default())
}

/// A window at an explicit size, for layout checks against the boards (1280×800).
fn open_sized(cx: &mut TestAppContext, dir: &Path, width: f32, height: f32) -> (Entity<CrowWindow>, VisualTestContext) {
    let bounds = Bounds::new(point(px(0.), px(0.)), size(px(width), px(height)));
    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
    open_with(cx, dir, options)
}

fn open_with(cx: &mut TestAppContext, dir: &Path, options: WindowOptions) -> (Entity<CrowWindow>, VisualTestContext) {
    let trash =
        dir.parent().unwrap_or(dir).join(format!(".trash-{}", dir.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::create_dir_all(&trash).expect("create test trash");
    cx.update(|cx| {
        crate::ui::init_for_tests(cx);
        crate::operations::init(cx, std::sync::Arc::new(crate::operations::FolderTrash(trash)));
        cx.bind_keys(crate::actions::finder_keymap());
        cx.bind_keys(crate::actions::vim_keymap());
        cx.bind_keys(crate::text_input::keymap());
        cx.bind_keys(crate::settings_window::keymap());
    });
    let dir = dir.to_path_buf();
    let window: WindowHandle<CrowWindow> = cx
        .update(|cx| cx.open_window(options, |window, cx| cx.new(|cx| CrowWindow::new(dir, false, None, window, cx))))
        .expect("open window");
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    let view = window.root(&mut cx).expect("root view");
    settle(&view, &mut cx);
    (view, cx)
}

/// Folders load on real threads; wait until every tab has finished.
fn settle(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) {
    for _ in 0..400 {
        cx.run_until_parked();
        let busy = view.read_with(cx, |w, cx| {
            w.tabs.iter().any(|(_, pane)| pane.read(cx).is_settling(cx)) || cx.global::<Operations>().running.is_some()
        });
        if !busy {
            cx.run_until_parked();
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("folders never finished loading");
}

/// Wait until the palette is not waiting on a background parent listing.
fn settle_palette(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) {
    for _ in 0..400 {
        cx.run_until_parked();
        let loading = view.read_with(cx, |w, _| {
            w.palette.as_ref().is_some_and(|palette| {
                palette.listing.rows.iter().any(|row| matches!(row, Row::Status(text) if text.as_ref() == "Loading…"))
            })
        });
        if !loading {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the palette never finished listing");
}

fn pane(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> Entity<Pane> {
    view.read_with(cx, |w, _| w.pane().clone())
}

fn selected(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> Option<String> {
    pane(view, cx).read_with(cx, |p, cx| p.cursor_entry(cx).map(|e| e.name))
}

fn dir(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> PathBuf {
    pane(view, cx).read_with(cx, |p, cx| p.current_dir(cx))
}

/// Whether the menu item for `action` would be enabled with the current focus.
fn action_available(action: &dyn gpui::Action, cx: &mut VisualTestContext) -> bool {
    let window = cx.update(|_, cx| cx.windows()[0]);
    cx.update_window(window, |_, window, cx| window.is_action_available(action, cx)).expect("check action")
}

/// The visible list rows as (name, indent depth).
fn rows(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> Vec<(String, usize)> {
    pane(view, cx).read_with(cx, |p, cx| p.rows(cx))
}

fn config_dir(cx: &VisualTestContext) -> PathBuf {
    cx.read(|cx| cx.global::<Ui>().config_dir.clone())
}

/// The escaping effects recorded since the last call, clearing them.
fn effect_calls(cx: &mut VisualTestContext) -> Vec<crate::effects::Call> {
    cx.update(|_, cx| crate::effects::recording(cx).expect("tests install recording effects").take_calls())
}

fn violations(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) -> Vec<crate::fuzz::Violation> {
    cx.update(|window, cx| view.read(cx).check_invariants(window, cx, false, None))
}

fn deep_violations(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) -> Vec<crate::fuzz::Violation> {
    cx.update(|window, cx| view.read(cx).check_invariants(window, cx, true, None))
}

#[gpui::test]
fn invariants_hold_after_open(cx: &mut TestAppContext) {
    let root = fixture("fuzz-invariants");
    let (view, mut cx) = open(cx, &root);
    let found = deep_violations(&view, &mut cx);
    assert!(found.is_empty(), "{found:?}");
}

#[gpui::test]
fn cursor_past_the_listing_fires(cx: &mut TestAppContext) {
    let root = fixture("fuzz-cursor");
    let (view, mut cx) = open(cx, &root);
    let pane = pane(&view, &cx);
    let folder = pane.read_with(&cx, |pane, _| pane.test_folder());
    folder.update(&mut cx, |folder, _| folder.set_cursor_ix_for_test(Some(999)));
    let found = violations(&view, &mut cx);
    assert!(found.iter().any(|violation| violation.check == "cursor"), "{found:?}");
}

#[gpui::test]
fn cursor_without_its_selection_fires(cx: &mut TestAppContext) {
    let root = fixture("fuzz-cursor-selection");
    let (view, mut cx) = open(cx, &root);
    let pane = pane(&view, &cx);
    let folder = pane.read_with(&cx, |pane, _| pane.test_folder());
    // Open selects row 0; move the cursor alone, as a highlight that skipped it would.
    folder.update(&mut cx, |folder, _| folder.set_cursor_ix_for_test(Some(1)));
    let found = violations(&view, &mut cx);
    assert!(found.iter().any(|violation| violation.check == "cursor"), "{found:?}");
}

#[gpui::test]
fn selection_outside_the_listing_fires(cx: &mut TestAppContext) {
    let root = fixture("fuzz-selection");
    let (view, mut cx) = open(cx, &root);
    let pane = pane(&view, &cx);
    let folder = pane.read_with(&cx, |pane, _| pane.test_folder());
    folder.update(&mut cx, |folder, _| folder.select_path_for_test(root.join("ghost.txt")));
    let found = violations(&view, &mut cx);
    assert!(found.iter().any(|violation| violation.check == "selection"), "{found:?}");
}

#[gpui::test]
fn inline_child_selection_is_in_the_listing(cx: &mut TestAppContext) {
    let root = fixture("fuzz-inline-selection");
    let (view, mut cx) = open(cx, &root);
    // Right expands alpha inline; down selects its child row.
    cx.simulate_keystrokes("right down");
    let found = violations(&view, &mut cx);
    assert!(found.is_empty(), "{found:?}");
}

#[gpui::test]
fn listing_mismatch_fires(cx: &mut TestAppContext) {
    let root = fixture("fuzz-listing");
    let (view, mut cx) = open(cx, &root);
    let pane = pane(&view, &cx);
    let folder = pane.read_with(&cx, |pane, _| pane.test_folder());
    folder.update(&mut cx, |folder, cx| folder.drop_entry_for_test("notes.txt", cx));
    let found = deep_violations(&view, &mut cx);
    assert!(found.iter().any(|violation| violation.check == "listing"), "{found:?}");
}

#[gpui::test]
fn focus_outside_the_surface_fires(cx: &mut TestAppContext) {
    let root = fixture("fuzz-focus");
    let (view, mut cx) = open(cx, &root);
    let stray = cx.update(|_, cx| cx.focus_handle());
    let found = cx.update(|window, cx| {
        window.focus(&stray, cx);
        view.read(cx).check_invariants(window, cx, false, None)
    });
    assert!(found.iter().any(|violation| violation.check == "focus"), "{found:?}");
}

/// H4 #8: ⌘N's second window must not fire the focus invariant.
#[gpui::test]
fn a_second_window_keeps_the_focus_invariant(cx: &mut TestAppContext) {
    let root = fixture("fuzz-inactive-focus");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-n");
    cx.run_until_parked();
    let found = check_all_windows(&mut cx, false, None);
    let windows = cx.update(|_, cx| cx.windows().len());
    assert_eq!(windows, 2, "cmd-n opened a second window");
    assert!(found.is_empty(), "windows={windows} {found:?}");
    drop(view);
}

/// H4 #20: the live run checks every browser window, and a window that is not
/// key may have no focused element (the Settings window or a second browser
/// window is key). The test fuzzer still requires focus.
#[gpui::test]
fn inactive_windows_may_have_no_focus(cx: &mut TestAppContext) {
    let root = fixture("fuzz-inactive-focus-blur");
    let (view, mut cx) = open(cx, &root);
    let (inactive, required) = cx.update(|window, cx| {
        window.blur(cx);
        assert!(window.focused(cx).is_none(), "the window has no focus");
        let view = view.read(cx);
        (
            view.check_invariants_with_focus(window, cx, false, None, false),
            view.check_invariants_with_focus(window, cx, false, None, true),
        )
    });
    assert!(!inactive.iter().any(|violation| violation.check == "focus"), "{inactive:?}");
    assert!(required.iter().any(|violation| violation.check == "focus"), "{required:?}");

    // GPUI keeps a deactivated window's focus handle; an inactive window may
    // still name a surface that is no longer allowed. Only the active window's
    // focus is checked.
    let (inactive, required) = cx.update(|window, cx| {
        let stray = cx.focus_handle();
        window.focus(&stray, cx);
        assert!(window.focused(cx).is_some(), "the window has a stray focus");
        let view = view.read(cx);
        (
            view.check_invariants_with_focus(window, cx, false, None, false),
            view.check_invariants_with_focus(window, cx, false, None, true),
        )
    });
    assert!(!inactive.iter().any(|violation| violation.check == "focus"), "{inactive:?}");
    assert!(required.iter().any(|violation| violation.check == "focus"), "{required:?}");
}

#[gpui::test]
fn filter_and_rename_keep_focus_in_the_pane(cx: &mut TestAppContext) {
    use crate::fuzz::Step;
    let tree = fuzz_tree("fuzz-filter-rename", 40);
    let found = replay(
        cx,
        &tree,
        40,
        crate::script![
            Step::Keys("/".into()),
            Step::Keys("enter".into()),
            Step::Keys("cmd-shift-n".into()),
            Step::Settle
        ],
    );
    assert!(found.is_empty(), "{found:?}");
}

#[gpui::test]
fn focus_outside_an_open_overlay_fires(cx: &mut TestAppContext) {
    let root = fixture("fuzz-overlay-focus");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    let pane_focus = pane(&view, &cx).read_with(&cx, |pane, _| pane.focus_handle().clone());
    let found = cx.update(|window, cx| {
        window.focus(&pane_focus, cx);
        view.read(cx).check_invariants(window, cx, false, None)
    });
    assert!(found.iter().any(|violation| violation.check == "focus"), "{found:?}");
}

/// Replace a config file and reload, as the file watcher would.
fn write_config(view: &Entity<CrowWindow>, cx: &mut VisualTestContext, name: &str, text: &str) {
    let dir = config_dir(cx);
    std::fs::create_dir_all(&dir).expect("config dir");
    std::fs::write(dir.join(name), text).expect("write config");
    cx.update(|_, cx| crate::ui::reload_config(cx));
    settle(view, cx);
}

#[gpui::test]
fn app_icon_setting_switches_the_dock_icon_live(cx: &mut TestAppContext) {
    use crow_core::AppIcon;
    let root = fixture("app-icon");
    let (view, mut cx) = open(cx, &root);
    let applied = |cx: &mut VisualTestContext| cx.update(|_, cx| cx.global::<Ui>().applied_icon);
    write_config(&view, &mut cx, "settings.toml", "app_icon = \"sun\"\n");
    assert_eq!(applied(&mut cx), Some(AppIcon::Sun));
    let calls = effect_calls(&mut cx);
    assert!(calls.contains(&crate::effects::Call::AppIcon(crate::ui::app_icon_png(AppIcon::Sun).len())), "{calls:?}");
    write_config(&view, &mut cx, "settings.toml", "app_icon = \"moon\"\n");
    assert_eq!(applied(&mut cx), Some(AppIcon::Sun), "a bad value keeps the last good icon");
    write_config(&view, &mut cx, "settings.toml", "theme = \"system\"\n");
    assert_eq!(applied(&mut cx), Some(AppIcon::Refined), "deleting the line goes back to the default");
    for icon in [AppIcon::Refined, AppIcon::Sun, AppIcon::Graphite, AppIcon::Bone] {
        assert!(crate::ui::app_icon_png(icon).starts_with(b"\x89PNG"), "{icon:?} is an embedded PNG");
    }
}

fn favorites_toml(paths: &[&Path]) -> String {
    let items: String = paths.iter().map(|p| format!("  {{ path = \"{}\" }},\n", p.display())).collect();
    format!("[[group]]\nname = \"Favorites\"\nitems = [\n{items}]\n[recent]\nshow = false\n")
}

fn sidebar_paths(cx: &VisualTestContext) -> Vec<PathBuf> {
    cx.read(Sidebar::paths)
}

/// What each selectable sidebar row shows, as the window renders it.
fn sidebar_looks(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) -> Vec<crate::sidebar::RowLook> {
    let sidebar = view.read_with(cx, |w, _| w.sidebar.clone());
    cx.update(|window, cx| sidebar.read_with(cx, |s, cx| s.looks_for_test(window, cx)))
}

/// Right-click sidebar row `i`, then pick the line running `id` (`None`: dismiss).
fn sidebar_menu_choose(view: &Entity<CrowWindow>, cx: &mut VisualTestContext, i: usize, id: Option<CommandId>) {
    let at = cx.debug_bounds(Box::leak(format!("sidebar-{i}").into_boxed_str())).expect("row on screen").center();
    cx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::default());
    let sidebar = view.read_with(cx, |w, _| w.sidebar.clone());
    let (row, kind) = sidebar.read_with(cx, |s, _| s.menu()).expect("a sidebar menu is open");
    assert_eq!(row, i, "the menu is about the clicked row");
    let ix = id.map(|id| {
        crate::context_menu::sidebar_slots(kind)
            .iter()
            .position(|slot| slot.is_some_and(|(slot_id, _)| slot_id == id))
            .expect("in the menu")
    });
    sidebar.update_in(cx, |s, window, cx| s.choose_menu_item(ix, window, cx));
    cx.run_until_parked();
    assert!(sidebar.read_with(cx, |s, _| s.menu().is_none()), "the menu closed");
}

#[gpui::test]
fn sidebar_context_menu_acts_on_the_clicked_favorite(cx: &mut TestAppContext) {
    use crate::context_menu::SidebarMenuKind;
    let root = fixture("sidebar-menu");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));

    // Dismissed: nothing changes, the window stays where it was.
    sidebar_menu_choose(&view, &mut cx, 1, None);
    assert_eq!(dir(&view, &cx), root);
    let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());

    sidebar_menu_choose(&view, &mut cx, 1, Some(CommandId::MoveFavoriteUp));
    settle(&view, &mut cx);
    assert_eq!(sidebar_paths(&cx), [root.join("beta"), root.join("alpha")], "beta moved up");

    sidebar_menu_choose(&view, &mut cx, 1, Some(CommandId::NewTab));
    settle(&view, &mut cx);
    assert_eq!(view.read_with(&cx, |w, _| w.tabs.len()), 2, "Open in New Tab opens a tab");
    assert_eq!(dir(&view, &cx), root.join("alpha"), "on the clicked folder");

    sidebar_menu_choose(&view, &mut cx, 0, Some(CommandId::CopyPath));
    let copied = cx.read(|cx| crate::effects::recording(cx).and_then(|recorder| recorder.text()));
    assert_eq!(copied.as_deref(), Some(root.join("beta").to_string_lossy().as_ref()));

    sidebar_menu_choose(&view, &mut cx, 0, Some(CommandId::Open));
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("beta"));

    sidebar_menu_choose(&view, &mut cx, 0, Some(CommandId::RemoveFromSidebar));
    settle(&view, &mut cx);
    assert_eq!(sidebar_paths(&cx), [root.join("alpha")], "beta left the sidebar");
    assert!(root.join("beta").is_dir(), "the folder itself stays");

    // Ctrl-click opens the same menu.
    let at = cx.debug_bounds("sidebar-0").expect("row on screen").center();
    cx.simulate_click(at, gpui::Modifiers::control());
    let menu = sidebar.read_with(&cx, |s, _| s.menu());
    assert_eq!(menu, Some((0, SidebarMenuKind::Favorite)));
    assert_eq!(dir(&view, &cx), root.join("beta"), "Ctrl-click does not open the folder");
}

#[gpui::test]
fn sidebar_context_menu_adds_a_recent_folder(cx: &mut TestAppContext) {
    use crate::context_menu::SidebarMenuKind;
    let root = fixture("sidebar-menu-recent");
    let (view, mut cx) = open(cx, &root);
    write_config(
        &view,
        &mut cx,
        "sidebar.toml",
        &favorites_toml(&[&root.join("alpha")]).replace("show = false", "show = true"),
    );
    pane(&view, &cx).update(&mut cx, |p, cx| p.navigate(root.join("beta"), cx));
    settle(&view, &mut cx);
    let mut recent = None;
    for _ in 0..400 {
        cx.run_until_parked();
        recent = sidebar_paths(&cx).iter().position(|p| *p == root.join("beta"));
        if recent.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let recent = recent.expect("beta is in Recent");
    let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());
    let at = cx.debug_bounds(Box::leak(format!("sidebar-{recent}").into_boxed_str())).expect("row").center();
    cx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::default());
    assert_eq!(sidebar.read_with(&cx, |s, _| s.menu()), Some((recent, SidebarMenuKind::Recent)));
    sidebar.update_in(&mut cx, |s, window, cx| s.choose_menu_item(None, window, cx));

    sidebar_menu_choose(&view, &mut cx, recent, Some(CommandId::AddToSidebar));
    settle(&view, &mut cx);
    let text = std::fs::read_to_string(config_dir(&cx).join("sidebar.toml")).expect("sidebar.toml");
    assert!(text.contains(&root.join("beta").display().to_string()), "beta is a favorite now: {text}");
}

#[gpui::test]
fn add_to_sidebar_adds_only_the_selected_folders(cx: &mut TestAppContext) {
    let root = fixture("add-sidebar-mixed");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-a ctrl-cmd-t");
    settle(&view, &mut cx);
    let paths = sidebar_paths(&cx);
    assert!(paths.contains(&root.join("alpha")), "the selected folder is added: {paths:?}");
    assert!(paths.contains(&root.join("beta")), "…and the other one");
    assert!(!paths.contains(&root.join("notes.txt")), "a selected file is not a sidebar row: {paths:?}");
    let text = std::fs::read_to_string(config_dir(&cx).join("sidebar.toml")).expect("sidebar.toml");
    for name in ["alpha", "beta"] {
        assert!(text.contains(&root.join(name).display().to_string()), "{name} is written: {text}");
    }
    assert!(!text.contains(&root.join("notes.txt").display().to_string()), "the file is not written: {text}");
}

#[gpui::test]
fn sidebar_looks_follow_rows(cx: &mut TestAppContext) {
    let root = fixture("looks");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
    let looks = sidebar_looks(&view, &mut cx);
    let labels: Vec<&str> = looks.iter().map(|look| look.label.as_str()).collect();
    assert_eq!(labels, ["alpha", "beta"]);
    assert!(looks.iter().all(|look| look.spoken == look.label), "no hint or mark yet: {looks:?}");
}

#[gpui::test]
fn sidebar_twins_get_their_parent_in_label_and_aria(cx: &mut TestAppContext) {
    let root = fixture("twins");
    std::fs::create_dir_all(root.join("one/git")).expect("create fixture");
    std::fs::create_dir_all(root.join("two/git")).expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("one/git"), &root.join("two/git")]));
    let looks = sidebar_looks(&view, &mut cx);
    let hints: Vec<Option<&str>> = looks.iter().map(|look| look.hint.as_deref()).collect();
    assert_eq!(hints, [Some("one"), Some("two")]);
    assert_eq!(looks[0].spoken, "git, in one");
    assert_eq!(looks[1].spoken, "git, in two");
}

#[gpui::test]
fn sidebar_twins_span_favorites_and_recent(cx: &mut TestAppContext) {
    let root = fixture("twins-recent");
    std::fs::create_dir_all(root.join("one/git")).expect("create fixture");
    std::fs::create_dir_all(root.join("two/git")).expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    write_config(
        &view,
        &mut cx,
        "sidebar.toml",
        &favorites_toml(&[&root.join("one/git")]).replace("show = false", "show = true"),
    );
    pane(&view, &cx).update(&mut cx, |pane, cx| pane.navigate(root.join("two/git"), cx));
    settle(&view, &mut cx);
    let looks = sidebar_looks(&view, &mut cx);
    let git: Vec<&crate::sidebar::RowLook> = looks.iter().filter(|look| look.label == "git").collect();
    assert_eq!(git.len(), 2, "one favorite, one recent: {looks:?}");
    assert_eq!(git[0].hint.as_deref(), Some("one"));
    assert_eq!(git[1].hint.as_deref(), Some("two"));
}

#[gpui::test]
fn sidebar_twins_walk_up(cx: &mut TestAppContext) {
    let root = fixture("twins-walk");
    std::fs::create_dir_all(root.join("x/a/git")).expect("create fixture");
    std::fs::create_dir_all(root.join("y/a/git")).expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("x/a/git"), &root.join("y/a/git")]));
    let looks = sidebar_looks(&view, &mut cx);
    let hints: Vec<Option<&str>> = looks.iter().map(|look| look.hint.as_deref()).collect();
    assert_eq!(hints, [Some("x"), Some("y")]);
}

#[gpui::test]
fn sidebar_twin_in_home_shows_a_tilde(cx: &mut TestAppContext) {
    let root = fixture("twins-home");
    std::fs::create_dir_all(root.join("git")).expect("create fixture");
    std::fs::create_dir_all(root.join("other/git")).expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.update(|_, cx| cx.global_mut::<Ui>().home = root.clone());
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("git"), &root.join("other/git")]));
    let looks = sidebar_looks(&view, &mut cx);
    assert_eq!(looks[0].hint.as_deref(), Some("~"));
    assert_eq!(looks[1].hint.as_deref(), Some("other"));
    assert_eq!(looks[0].spoken, format!("git, in {}", root.file_name().unwrap().to_string_lossy()));
}

#[gpui::test]
fn sidebar_cursor_is_a_ring_and_only_the_current_folder_is_filled(cx: &mut TestAppContext) {
    let root = fixture("ring");
    let (view, mut cx) = open(cx, &root.join("alpha"));
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));

    let looks = sidebar_looks(&view, &mut cx);
    assert!(looks[0].current && !looks[0].ring, "unfocused: the current folder is filled: {looks:?}");
    assert!(!looks[1].current && !looks[1].ring);

    cx.simulate_keystrokes("tab");
    let looks = sidebar_looks(&view, &mut cx);
    assert!(looks[0].current && looks[0].ring, "cursor 0 on the current folder: fill and ring");
    assert!(!looks[1].ring);

    cx.simulate_keystrokes("down");
    settle(&view, &mut cx);
    let looks = sidebar_looks(&view, &mut cx);
    assert!(!looks[0].current && !looks[0].ring, "the fill left alpha with the pane");
    assert!(looks[1].current && looks[1].ring, "beta is current and carries the cursor ring");

    cx.simulate_keystrokes("tab");
    let looks = sidebar_looks(&view, &mut cx);
    assert!(looks.iter().all(|look| !look.ring), "no ring while the list has focus");
    assert!(looks[1].current, "the current folder stays filled");
}

#[gpui::test]
fn sidebar_vim_j_moves_the_ring(cx: &mut TestAppContext) {
    let root = fixture("ring-vim");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
    use_vim(&view, &mut cx);
    cx.simulate_keystrokes("tab j");
    let looks = sidebar_looks(&view, &mut cx);
    assert!(looks[1].ring && !looks[0].ring, "j moves the ring: {looks:?}");
}

#[gpui::test]
fn disk_row_shows_free_space_and_keeps_its_aria_label(cx: &mut TestAppContext) {
    let root = fixture("disk");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[]));
    cx.update(|_, cx| {
        cx.global_mut::<Ui>().volumes = vec![crow_macos::Volume {
            name: "Test HD".into(),
            path: root.clone(),
            total_bytes: 494_384_795_648,
            free_bytes: 182_400_000_000,
            ejectable: true,
        }];
        cx.refresh_windows();
    });
    cx.run_until_parked();
    let looks = sidebar_looks(&view, &mut cx);
    let disk = looks.iter().find(|look| look.label == "Test HD").expect("volume row");
    assert_eq!(disk.right.as_deref(), Some("182 GB free"));
    let (used, tone) = disk.bar.expect("a bar for a known total");
    assert!((used - 0.631).abs() < 0.001, "used fraction {used}");
    assert_eq!(tone, crate::sidebar::BarTone::Neutral);
    assert_eq!(disk.spoken, "Test HD, 182.4 GB free of 494.4 GB, ejectable");

    cx.update(|_, cx| {
        cx.global_mut::<Ui>().volumes[0].free_bytes = 24_719_239_782;
        cx.refresh_windows();
    });
    cx.run_until_parked();
    let looks = sidebar_looks(&view, &mut cx);
    let disk = looks.iter().find(|look| look.label == "Test HD").expect("volume row");
    assert_eq!(disk.bar.expect("a bar").1, crate::sidebar::BarTone::Warning);
    assert!(disk.spoken.ends_with(", ejectable, almost full"), "{}", disk.spoken);
}

#[gpui::test]
fn vim_marks_show_as_letters_in_the_sidebar(cx: &mut TestAppContext) {
    let root = fixture("marks-letters");
    let (view, mut cx) = open(cx, &root);
    let toml = format!(
        "[[group]]\nname = \"Favorites\"\nitems = [\n  {{ path = \"{}\", key = \"a\" }},\n  {{ path = \"{}\" }},\n]\n[recent]\nshow = false\n",
        root.join("alpha").display(),
        root.join("beta").display()
    );
    write_config(&view, &mut cx, "sidebar.toml", &toml);
    let looks = sidebar_looks(&view, &mut cx);
    assert!(looks.iter().all(|look| look.right.is_none()), "the Finder layer shows no letters: {looks:?}");

    use_vim(&view, &mut cx);
    let looks = sidebar_looks(&view, &mut cx);
    assert_eq!(looks[0].right.as_deref(), Some("a"));
    assert!(looks[1].right.is_none(), "only the marked favorite shows a letter");

    cx.simulate_keystrokes("down cmd-down");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("beta"));
    cx.simulate_keystrokes("m a");
    settle(&view, &mut cx);
    let looks = sidebar_looks(&view, &mut cx);
    assert!(looks[0].right.is_none(), "the mark moved away from alpha");
    assert_eq!(looks[1].right.as_deref(), Some("a"), "beta now shows the mark");
}

#[gpui::test]
fn symlinked_twin_says_alias(cx: &mut TestAppContext) {
    let root = fixture("alias");
    std::fs::create_dir_all(root.join("real/git")).expect("create fixture");
    std::fs::create_dir_all(root.join("link")).expect("create fixture");
    std::os::unix::fs::symlink(root.join("real/git"), root.join("link/git")).expect("create symlink");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("real/git"), &root.join("link/git")]));
    for _ in 0..400 {
        cx.run_until_parked();
        if cx.update(|_, cx| cx.global::<Ui>().aliases.contains(&root.join("link/git"))) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let looks = sidebar_looks(&view, &mut cx);
    assert_eq!(looks[0].hint.as_deref(), Some("real"));
    assert_eq!(looks[1].hint.as_deref(), Some("link · alias"));
    assert_eq!(looks[1].spoken, "git, in link, alias");
}

#[gpui::test]
fn same_keys_mean_different_things_in_sidebar_and_list(cx: &mut TestAppContext) {
    let root = fixture("contexts");
    std::fs::write(root.join("beta/b.txt"), "").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"));

    cx.simulate_keystrokes("down");
    assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "Down in the list moves the selection");

    cx.simulate_keystrokes("tab down");
    settle(&view, &mut cx);
    let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());
    assert_eq!(sidebar.read_with(&cx, |s, _| s.cursor()), 1, "Down in the sidebar moves its cursor");
    assert_eq!(dir(&view, &cx), root.join("beta"), "…and shows beta in the center");
    assert_eq!(selected(&view, &cx).as_deref(), Some("b.txt"), "the list follows the sidebar cursor");

    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("beta"), "Return in the sidebar opens the favorite");
    let list_focus = pane(&view, &cx).read_with(&cx, |p, _| p.focus_handle().clone());
    assert!(cx.update(|window, _| list_focus.is_focused(window)), "…and moves focus to the list");

    cx.simulate_keystrokes("enter");
    assert!(pane(&view, &cx).read_with(&cx, |p, _| p.is_renaming()), "Return in the list means rename");
}

#[gpui::test]
fn sidebar_arrows_show_the_folder_and_keep_focus(cx: &mut TestAppContext) {
    let root = fixture("sidebar-show");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
    assert_eq!(dir(&view, &cx), root);

    cx.simulate_keystrokes("tab");
    assert_eq!(dir(&view, &cx), root, "tabbing into the sidebar does not navigate");
    cx.simulate_keystrokes("down");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("beta"), "Down shows the cursor's folder");
    assert!(sidebar_focused(&view, &mut cx), "…and focus stays in the sidebar");

    cx.simulate_keystrokes("up");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"));
    assert!(sidebar_focused(&view, &mut cx));

    cx.simulate_keystrokes("tab");
    assert!(!sidebar_focused(&view, &mut cx), "Tab moves to the center");
    assert_eq!(dir(&view, &cx), root.join("alpha"), "…which already shows the cursor's folder");
}

#[gpui::test]
fn sidebar_return_and_right_focus_the_center(cx: &mut TestAppContext) {
    let root = fixture("sidebar-open");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));

    cx.simulate_keystrokes("tab down");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("beta"));
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert!(!sidebar_focused(&view, &mut cx), "Return focuses the center");
    assert_eq!(dir(&view, &cx), root.join("beta"));

    cx.simulate_keystrokes("tab");
    assert!(sidebar_focused(&view, &mut cx));
    cx.simulate_keystrokes("right");
    assert!(!sidebar_focused(&view, &mut cx), "→ focuses the center too");
    assert_eq!(dir(&view, &cx), root.join("beta"), "…without navigating again");
}

#[gpui::test]
fn sidebar_arrow_closes_search(cx: &mut TestAppContext) {
    let root = fixture("sidebar-search");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("notes");
    wait_for_search(&view, &mut cx);
    cx.simulate_keystrokes("tab tab");
    assert!(sidebar_focused(&view, &mut cx));

    cx.simulate_keystrokes("down");
    settle(&view, &mut cx);
    assert!(view.read_with(&cx, |w, _| w.search.is_none()), "a sidebar move closes search");
    assert_eq!(dir(&view, &cx), root.join("beta"));
    assert!(sidebar_focused(&view, &mut cx), "focus stays in the sidebar");
}

#[gpui::test]
fn sidebar_tab_in_does_not_navigate(cx: &mut TestAppContext) {
    let root = fixture("sidebar-tab");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
    let before = dir(&view, &cx);
    cx.simulate_keystrokes("tab");
    assert_eq!(dir(&view, &cx), before);
}

#[gpui::test]
fn option_arrows_jump_between_sidebar_groups(cx: &mut TestAppContext) {
    let root = fixture("groups");
    let (view, mut cx) = open(cx, &root);
    let toml = format!(
        "[[group]]\nname = \"One\"\nitems = [ {{ path = \"{}\" }} ]\n[[group]]\nname = \"Two\"\nitems = [ {{ path = \"{}\" }} ]\n[recent]\nshow = true\ncount = 6\n[locations]\nshow = false\n",
        root.join("alpha").display(),
        root.join("beta").display()
    );
    write_config(&view, &mut cx, "sidebar.toml", &toml);
    assert_eq!(sidebar_paths(&cx), [root.join("alpha"), root.join("beta"), root.clone()], "two groups, then Recent");

    cx.simulate_keystrokes("tab alt-down");
    settle(&view, &mut cx);
    let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());
    assert_eq!(sidebar.read_with(&cx, |s, _| s.cursor()), 1, "the first row of the next group");
    assert_eq!(dir(&view, &cx), root.join("beta"));

    cx.simulate_keystrokes("alt-down");
    settle(&view, &mut cx);
    assert_eq!(sidebar.read_with(&cx, |s, _| s.cursor()), 2, "Recent is a group too");
    assert_eq!(dir(&view, &cx), root);

    cx.simulate_keystrokes("alt-up");
    settle(&view, &mut cx);
    assert_eq!(sidebar.read_with(&cx, |s, _| s.cursor()), 1);
    cx.simulate_keystrokes("alt-up");
    settle(&view, &mut cx);
    assert_eq!(sidebar.read_with(&cx, |s, _| s.cursor()), 0);
}

#[gpui::test]
fn walking_recent_keeps_the_cursor_on_the_shown_folder(cx: &mut TestAppContext) {
    let root = fixture("recent-walk");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[]).replace("show = false", "show = true"));
    // beta ranks above alpha, which ranks above root: a visit to alpha would tie it with beta.
    for d in ["alpha", "beta", "alpha", "beta", "beta"] {
        pane(&view, &cx).update(&mut cx, |pane, cx| pane.navigate(root.join(d), cx));
        settle(&view, &mut cx);
    }
    assert_eq!(sidebar_paths(&cx), [root.join("beta"), root.join("alpha"), root.clone()]);
    cx.simulate_keystrokes("tab");
    for (step, want) in [root.join("alpha"), root.clone()].into_iter().enumerate() {
        cx.simulate_keystrokes("down");
        settle(&view, &mut cx);
        assert_eq!(dir(&view, &cx), want);
        let looks = sidebar_looks(&view, &mut cx);
        assert!(looks[step + 1].ring && looks[step + 1].current, "Recent kept its order under the cursor: {looks:?}");
    }
    assert_eq!(sidebar_paths(&cx), [root.join("beta"), root.join("alpha"), root.clone()], "the walk re-ranked nothing");
}

#[gpui::test]
fn a_sidebar_walk_counts_only_the_folder_you_leave_on(cx: &mut TestAppContext) {
    let root = fixture("walk-visits");
    std::fs::create_dir_all(root.join("gamma")).expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    write_config(
        &view,
        &mut cx,
        "sidebar.toml",
        &favorites_toml(&[&root.join("alpha"), &root.join("beta"), &root.join("gamma")]),
    );
    let visited = |cx: &mut VisualTestContext| cx.update(|_, cx| cx.global::<Ui>().palette_recents.clone());
    cx.simulate_keystrokes("tab down down");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("gamma"));
    let during = visited(&mut cx);
    assert!(!during.contains(&root.join("beta")) && !during.contains(&root.join("gamma")), "{during:?}");

    cx.simulate_keystrokes("tab");
    settle(&view, &mut cx);
    let after = visited(&mut cx);
    assert!(after.contains(&root.join("gamma")), "leaving the sidebar counts gamma: {after:?}");
    assert!(!after.contains(&root.join("beta")), "beta was only passed: {after:?}");
}

#[gpui::test]
fn sidebar_arrow_walk_is_one_history_step(cx: &mut TestAppContext) {
    let root = fixture("sidebar-history");
    std::fs::create_dir_all(root.join("gamma")).expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    write_config(
        &view,
        &mut cx,
        "sidebar.toml",
        &favorites_toml(&[&root.join("alpha"), &root.join("beta"), &root.join("gamma")]),
    );
    cx.simulate_keystrokes("tab down down");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("gamma"));
    cx.simulate_keystrokes("cmd-[");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root, "Back returns to where the walk began");
}

#[gpui::test]
fn favorites_reorder_remove_and_add_from_the_keyboard(cx: &mut TestAppContext) {
    let root = fixture("favorites");
    let (view, mut cx) = open(cx, &root);
    let (alpha, beta) = (root.join("alpha"), root.join("beta"));
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&alpha, &beta]));
    assert_eq!(sidebar_paths(&cx), [alpha.clone(), beta.clone()]);

    cx.simulate_keystrokes("tab cmd-alt-down");
    settle(&view, &mut cx);
    assert_eq!(sidebar_paths(&cx), [beta.clone(), alpha.clone()], "Cmd-Option-Down moves the favorite down");
    let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());
    assert_eq!(sidebar.read_with(&cx, |s, _| s.cursor()), 1, "…and the cursor goes with it");
    let text = std::fs::read_to_string(config_dir(&cx).join("sidebar.toml")).unwrap();
    assert!(text.find("beta").unwrap() < text.find("alpha").unwrap(), "saved in sidebar.toml: {text}");

    cx.simulate_keystrokes("cmd-backspace");
    settle(&view, &mut cx);
    assert_eq!(sidebar_paths(&cx), std::slice::from_ref(&beta), "Cmd-Delete removes it from the sidebar");
    assert!(alpha.is_dir(), "…and only from the sidebar");

    // In the list, Ctrl-Cmd-T adds the selected folder (Finder's Add to Sidebar).
    cx.simulate_keystrokes("tab");
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"));
    cx.simulate_keystrokes("ctrl-cmd-t");
    settle(&view, &mut cx);
    assert_eq!(sidebar_paths(&cx), [beta, alpha]);
}

#[gpui::test]
fn dragging_a_favorite_down_inserts_below_the_row(cx: &mut TestAppContext) {
    let root = fixture("favorite-drag");
    let (view, mut cx) = open(cx, &root);
    let (alpha, beta) = (root.join("alpha"), root.join("beta"));
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&alpha, &beta]));
    let from = cx.debug_bounds("sidebar-0").expect("first favorite row");
    let to = cx.debug_bounds("sidebar-1").expect("second favorite row");
    let start = gpui::point(from.left() + gpui::px(20.), from.top() + gpui::px(6.));
    let over = gpui::point(to.left() + gpui::px(20.), to.bottom() - gpui::px(3.));

    cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(over, Some(gpui::MouseButton::Left), gpui::Modifiers::default());
    let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());
    assert_eq!(sidebar.read_with(&cx, |s, _| s.insert_slot()), Some((0, 2)), "one slot below beta");
    cx.simulate_mouse_up(over, gpui::MouseButton::Left, gpui::Modifiers::default());
    settle(&view, &mut cx);
    assert_eq!(sidebar_paths(&cx), [beta.clone(), alpha.clone()], "the lower half of a row drops after it");

    // And the upper half of a row is the single slot before it.
    let alpha_row = cx.debug_bounds("sidebar-1").expect("alpha row");
    let start = gpui::point(alpha_row.left() + gpui::px(20.), alpha_row.center().y);
    let beta_row = cx.debug_bounds("sidebar-0").expect("beta row");
    let top = gpui::point(beta_row.left() + gpui::px(20.), beta_row.top() + gpui::px(6.));
    cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(top, Some(gpui::MouseButton::Left), gpui::Modifiers::default());
    let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());
    assert_eq!(sidebar.read_with(&cx, |s, _| s.insert_slot()), Some((0, 0)), "one slot above alpha");
    cx.simulate_mouse_up(top, gpui::MouseButton::Left, gpui::Modifiers::default());
    settle(&view, &mut cx);
    assert_eq!(sidebar_paths(&cx), [alpha, beta], "…and dropping there moves it back");
}

#[gpui::test]
fn dropping_on_a_favorite_in_full_motion_pulses_it(cx: &mut TestAppContext) {
    let root = fixture("drop-favorite-full");
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "full");
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("beta")]));
    let sidebar = view.read_with(&cx, |window, _| window.sidebar.clone());
    drop_on(&mut cx, "sidebar-0", vec![root.join("notes.txt")]);
    for _ in 0..200 {
        if sidebar.read_with(&cx, |sidebar, _| sidebar.receiving(&root.join("beta"))) {
            break;
        }
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(sidebar.read_with(&cx, |sidebar, _| sidebar.receiving(&root.join("beta"))), "the favorite pulses");
    settle(&view, &mut cx);
    assert!(root.join("beta/notes.txt").exists());
}

#[gpui::test]
fn dropping_on_a_favorite_moves_into_it(cx: &mut TestAppContext) {
    let root = fixture("drop-favorite");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("beta")]));
    drop_on(&mut cx, "sidebar-0", vec![root.join("notes.txt")]);
    settle(&view, &mut cx);
    assert!(root.join("beta/notes.txt").exists());

    assert_eq!(sidebar_paths(&cx), [root.join("beta")], "the middle of a row is not an insertion point");

    // Folders dropped on a group's title become favorites.
    drop_on(&mut cx, "sidebar-header-Favorites", vec![root.join("alpha")]);
    settle(&view, &mut cx);
    assert_eq!(sidebar_paths(&cx), [root.join("beta"), root.join("alpha")]);
    assert!(root.join("alpha").is_dir(), "added, not moved");
}

#[gpui::test]
fn dropping_between_favorites_adds_a_favorite_there(cx: &mut TestAppContext) {
    let root = fixture("drop-between");
    for name in ["gamma", "delta"] {
        std::fs::create_dir_all(root.join(name)).expect("create fixture");
    }
    let (view, mut cx) = open(cx, &root);
    let (alpha, beta, gamma, delta) = (root.join("alpha"), root.join("beta"), root.join("gamma"), root.join("delta"));
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&alpha, &beta]));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());

    // A folder from another app over the lower edge of the first favorite: the line
    // marks the boundary before the second, and no row takes the drop fill.
    let first = cx.debug_bounds("sidebar-0").expect("first favorite row");
    let edge = gpui::point(first.left() + gpui::px(20.), first.bottom() - gpui::px(2.));
    let paths = gpui::ExternalPaths(vec![gamma.clone()].into());
    cx.simulate_event(gpui::FileDropEvent::Entered { position: edge, paths });
    cx.simulate_event(gpui::FileDropEvent::Pending { position: edge });
    cx.run_until_parked();
    assert_eq!(sidebar.read_with(&cx, |s, _| s.insert_slot()), Some((0, 1)), "between alpha and beta");
    assert_eq!(sidebar.read_with(&cx, |s, _| s.drop_target()), None, "no row is the target");
    assert!(sidebar_looks(&view, &mut cx).iter().all(|look| !look.drop));
    cx.simulate_event(gpui::FileDropEvent::Submit { position: edge });
    settle(&view, &mut cx);
    assert_eq!(sidebar_paths(&cx), [alpha.clone(), gamma.clone(), beta.clone()], "added at that place");
    assert!(gamma.is_dir() && !alpha.join("gamma").exists(), "added, not moved");

    // A folder dragged from the list onto the upper edge of the first favorite goes first.
    let from = cx.debug_bounds("row-delta").expect("delta row").center();
    let first = cx.debug_bounds("sidebar-0").expect("first favorite row");
    let edge = gpui::point(first.left() + gpui::px(20.), first.top() + gpui::px(2.));
    cx.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(edge, Some(gpui::MouseButton::Left), gpui::Modifiers::default());
    assert_eq!(sidebar.read_with(&cx, |s, _| s.insert_slot()), Some((0, 0)), "above alpha");
    cx.simulate_mouse_up(edge, gpui::MouseButton::Left, gpui::Modifiers::default());
    settle(&view, &mut cx);
    assert_eq!(sidebar_paths(&cx), [delta.clone(), alpha.clone(), gamma, beta]);
    assert!(delta.is_dir(), "added, not moved");

    // A file can't be a favorite: dropped between rows it is left where it is.
    let last = cx.debug_bounds("sidebar-3").expect("last favorite row");
    drop_at(
        &mut cx,
        gpui::point(last.left() + gpui::px(20.), last.bottom() - gpui::px(2.)),
        vec![root.join("notes.txt")],
    );
    settle(&view, &mut cx);
    assert_eq!(sidebar_paths(&cx).len(), 4);
    assert!(root.join("notes.txt").exists(), "not moved into the folder next to the line");
}

#[gpui::test]
fn recent_lists_visited_folders_that_are_not_favorites(cx: &mut TestAppContext) {
    let root = fixture("recent");
    let (view, mut cx) = open(cx, &root);
    write_config(
        &view,
        &mut cx,
        "sidebar.toml",
        &favorites_toml(&[&root.join("beta")]).replace("show = false", "show = true"),
    );
    cx.simulate_keystrokes("cmd-down");
    settle(&view, &mut cx);
    let paths = sidebar_paths(&cx);
    assert!(paths.contains(&root.join("alpha")), "visited: {paths:?}");
    assert_eq!(paths.iter().filter(|p| **p == root.join("beta")).count(), 1, "favorites aren't repeated");
    let state = cx.update(|_, cx| cx.global::<Ui>().state_dir.clone());
    assert!(state.join("recent.json").exists(), "saved for next time");
}

#[gpui::test]
fn recent_leaves_out_favorites_reached_through_a_symlink(cx: &mut TestAppContext) {
    let root = fixture("recent-symlink");
    std::os::unix::fs::symlink(&root, root.join("link")).expect("create symlink");
    let (view, mut cx) = open(cx, &root);
    write_config(
        &view,
        &mut cx,
        "sidebar.toml",
        &favorites_toml(&[&root.join("alpha")]).replace("show = false", "show = true"),
    );
    for dir in ["link/alpha", "link/beta", "beta"] {
        pane(&view, &cx).update(&mut cx, |p, cx| p.navigate(root.join(dir), cx));
        settle(&view, &mut cx);
    }
    let mut paths = Vec::new();
    for _ in 0..400 {
        cx.run_until_parked();
        paths = sidebar_paths(&cx);
        if paths.iter().any(|p| p.ends_with("beta")) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!paths.contains(&root.join("link/alpha")), "alpha is a favorite, by either path: {paths:?}");
    let betas = paths.iter().filter(|p| p.ends_with("beta")).count();
    assert_eq!(betas, 1, "one beta, though it was visited by two paths: {paths:?}");
}

#[gpui::test]
fn view_changes_are_remembered_per_folder(cx: &mut TestAppContext) {
    let root = fixture("views");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3");
    settle(&view, &mut cx);
    let text = std::fs::read_to_string(config_dir(&cx).join("views.toml")).unwrap();
    assert!(text.contains("view = \"columns\""), "saved as this folder's own setting: {text}");

    // A folder without its own setting shows the defaults…
    let mode = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, _| p.mode());
    pane(&view, &cx).update(&mut cx, |p, cx| p.navigate(root.join("beta"), cx));
    settle(&view, &mut cx);
    assert_eq!(mode(&cx), ViewMode::List);
    // …and going back shows the remembered one.
    cx.simulate_keystrokes("cmd-[");
    settle(&view, &mut cx);
    assert_eq!(mode(&cx), ViewMode::Columns);
}

#[gpui::test]
fn view_per_folder_off_keeps_the_window_view(cx: &mut TestAppContext) {
    let root = fixture("views-locked");
    let (view, mut cx) = open(cx, &root);
    let mode = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, _| p.mode());
    write_config(&view, &mut cx, "settings.toml", "view_per_folder = false\n");
    let views = format!("[folder.\"{}\"]\nview = \"columns\"\n", root.join("beta").display());
    write_config(&view, &mut cx, "views.toml", &views);

    pane(&view, &cx).update(&mut cx, |p, cx| p.navigate(root.join("beta"), cx));
    settle(&view, &mut cx);
    assert_eq!(mode(&cx), ViewMode::List, "the folder's own view is ignored");

    cx.simulate_keystrokes("cmd-3");
    settle(&view, &mut cx);
    assert_eq!(mode(&cx), ViewMode::Columns);
    let text = std::fs::read_to_string(config_dir(&cx).join("views.toml")).unwrap();
    assert_eq!(text, views, "Cmd-3 saves nothing");

    cx.simulate_keystrokes("cmd-[");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root);
    assert_eq!(mode(&cx), ViewMode::Columns, "going back keeps the window's view");

    cx.simulate_keystrokes("cmd-t");
    settle(&view, &mut cx);
    assert_eq!(mode(&cx), ViewMode::Columns, "a new tab copies its window");

    cx.simulate_keystrokes("cmd-2");
    settle(&view, &mut cx);
    write_config(&view, &mut cx, "settings.toml", "view_per_folder = true\n");
    pane(&view, &cx).update(&mut cx, |p, cx| p.navigate(root.join("beta"), cx));
    settle(&view, &mut cx);
    assert_eq!(mode(&cx), ViewMode::Columns, "turned back on, folders decide again");
}

#[gpui::test]
fn toolbar_view_buttons_switch_views(cx: &mut TestAppContext) {
    let root = fixture("toolbar-views");
    let (view, mut cx) = open(cx, &root);
    let mode = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, _| p.mode());
    assert_eq!(mode(&cx), ViewMode::List);
    let columns = cx.debug_bounds("view-columns").expect("columns button");
    cx.simulate_click(columns.center(), gpui::Modifiers::default());
    settle(&view, &mut cx);
    assert_eq!(mode(&cx), ViewMode::Columns);
    let list = cx.debug_bounds("view-list").expect("list button");
    cx.simulate_click(list.center(), gpui::Modifiers::default());
    settle(&view, &mut cx);
    assert_eq!(mode(&cx), ViewMode::List);
}

#[gpui::test]
fn back_button_tooltip_names_the_folder(cx: &mut TestAppContext) {
    let root = fixture("back-tooltip");
    let executor = cx.executor();
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-down");
    settle(&view, &mut cx);
    let back = view.read_with(&cx, |w, cx| w.pane().read(cx).back_target().map(Path::to_path_buf));
    assert_eq!(back.as_deref(), Some(root.as_path()), "Back points at the folder we came from");

    let bounds = cx.debug_bounds("back").expect("the back button is laid out");
    cx.simulate_mouse_move(bounds.center(), None, gpui::Modifiers::default());
    executor.advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    cx.simulate_mouse_move(bounds.center(), None, gpui::Modifiers::default());
    assert!(cx.debug_bounds("tooltip").is_some(), "the tooltip is drawn after the 500 ms delay");
}

#[gpui::test]
fn path_bar_segments_navigate_as_history_steps(cx: &mut TestAppContext) {
    let root = fixture("path-bar");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-down");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"));

    // The bar ends in the selected item, so its first segment is `/`'s own entry.
    let bounds = cx.debug_bounds("path-segment-0").expect("path bar segment is laid out");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), PathBuf::from("/"), "clicking a segment opens that folder");
    cx.simulate_keystrokes("cmd-[");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"), "and it left a history step to go back to");
}

/// The bar's data for the current frame.
fn bar_state(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) -> BarState {
    let window = cx.update(|_, cx| cx.windows()[0]);
    cx.update_window(window, |_, window, cx| view.read(cx).bar_state(window, cx)).expect("read the bar")
}

fn primary_text(state: &BarState) -> String {
    match &state.primary {
        Primary::Pending(text) => (*text).to_owned(),
        Primary::Message(text) => text.to_string(),
        Primary::Running(running) => format!("{} — {}", running.label, running.detail),
        Primary::Hints(_) => String::new(),
    }
}

fn hint_labels(state: &BarState) -> Vec<String> {
    match &state.primary {
        Primary::Hints(hints) => hints.iter().map(|hint| hint.label.to_owned()).collect(),
        _ => Vec::new(),
    }
}

fn hint_keys(state: &BarState) -> Vec<String> {
    match &state.primary {
        Primary::Hints(hints) => hints.iter().map(|hint| hint.keys.to_string()).collect(),
        _ => Vec::new(),
    }
}

#[gpui::test]
fn hints_follow_focus(cx: &mut TestAppContext) {
    let root = fixture("bar-hints-focus");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));

    let state = bar_state(&view, &mut cx);
    assert_eq!(hint_labels(&state), ["filter"], "⌘F and ⌘K live on the toolbar");
    assert_eq!(hint_keys(&state), ["/"]);

    cx.simulate_keystrokes("/");
    settle(&view, &mut cx);
    let state = bar_state(&view, &mut cx);
    assert_eq!(hint_labels(&state), ["to matches", "clear", "search deeper"]);
    assert_eq!(hint_keys(&state), ["↩", "esc", "⌘F"]);

    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("tab");
    settle(&view, &mut cx);
    let state = bar_state(&view, &mut cx);
    assert_eq!(hint_labels(&state), ["groups", "to list", "reorder", "remove"]);
    assert_eq!(hint_keys(&state), ["⌥↑↓", "⇥→", "⌥⌘↑↓", "⌘⌫"]);

    cx.simulate_keystrokes("tab");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("a");
    cx.simulate_keystrokes("tab");
    settle(&view, &mut cx);
    let state = bar_state(&view, &mut cx);
    assert_eq!(hint_labels(&state), ["open", "reveal", "Quick Look"]);
    assert_eq!(hint_keys(&state), ["↩", "⌘R", "Space"]);
}

#[gpui::test]
fn sidebar_hints_at_1280_show_groups_and_the_right_arrow(cx: &mut TestAppContext) {
    let root = fixture("bar-hints-sidebar-1280");
    let (view, mut cx) = open_sized(cx, &root, 1280., 800.);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
    cx.simulate_keystrokes("tab");
    settle(&view, &mut cx);

    let state = bar_state(&view, &mut cx);
    assert_eq!(hint_labels(&state), ["groups", "to list", "reorder", "remove"]);
    assert_eq!(hint_keys(&state), ["⌥↑↓", "⇥→", "⌥⌘↑↓", "⌘⌫"]);
}

#[gpui::test]
fn vim_layer_hints_use_the_vim_context(cx: &mut TestAppContext) {
    let root = fixture("bar-hints-vim");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
    use_vim(&view, &mut cx);

    let state = bar_state(&view, &mut cx);
    assert_eq!(hint_labels(&state), ["filter"]);
    cx.simulate_keystrokes("tab");
    settle(&view, &mut cx);
    let state = bar_state(&view, &mut cx);
    assert_eq!(hint_labels(&state), ["groups", "to list", "reorder", "remove"]);
    assert_eq!(hint_keys(&state), ["⌥↑↓", "⇥l", "⌥⌘↑↓", "⌘⌫"], "the Vim layer's l opens the list");

    cx.simulate_keystrokes("tab m");
    settle(&view, &mut cx);
    let state = bar_state(&view, &mut cx);
    assert!(matches!(state.primary, Primary::Pending(_)), "the mark prompt replaces hints");
    assert!(hint_labels(&state).is_empty());
}

#[gpui::test]
fn idle_hints_retire_after_three_uses(cx: &mut TestAppContext) {
    let root = fixture("bar-hints-retire");
    let (view, mut cx) = open(cx, &root);
    for _ in 0..3 {
        cx.simulate_keystrokes("/");
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
    }
    let state = bar_state(&view, &mut cx);
    assert!(hint_labels(&state).is_empty(), "the used hint retired");

    cx.simulate_keystrokes("/");
    settle(&view, &mut cx);
    let state = bar_state(&view, &mut cx);
    assert_eq!(hint_labels(&state), ["to matches", "clear", "search deeper"], "focus hints never retire");
}

#[gpui::test]
fn retired_hints_stay_retired_after_reload(cx: &mut TestAppContext) {
    let root = fixture("bar-hints-reload");
    let (view, mut cx) = open(cx, &root);
    for _ in 0..3 {
        cx.simulate_keystrokes("/");
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
    }
    cx.update(|_, cx| crate::usage::reload_from_disk(cx));
    write_config(&view, &mut cx, "settings.toml", "theme = \"system\"\n");
    let state = bar_state(&view, &mut cx);
    assert!(hint_labels(&state).is_empty(), "the count came back from usage.json");
}

#[gpui::test]
fn hints_always_never_retire(cx: &mut TestAppContext) {
    let root = fixture("bar-hints-always");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "settings.toml", "hints = \"always\"\n");
    for _ in 0..3 {
        cx.simulate_keystrokes("/");
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
    }
    let state = bar_state(&view, &mut cx);
    assert_eq!(hint_labels(&state), ["filter"]);
}

#[gpui::test]
fn hints_off_hides_every_hint(cx: &mut TestAppContext) {
    let root = fixture("bar-hints-off");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "settings.toml", "keymap = \"vim\"\nhints = \"off\"\n");
    let state = bar_state(&view, &mut cx);
    assert!(hint_labels(&state).is_empty());

    cx.simulate_keystrokes("m");
    settle(&view, &mut cx);
    let state = bar_state(&view, &mut cx);
    assert!(matches!(state.primary, Primary::Pending(_)), "the mark prompt is not a hint");
}

#[gpui::test]
fn error_drops_hints(cx: &mut TestAppContext) {
    let root = fixture("bar-hints-error");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "views.toml", "[defaults]\nsort = \"colour\"\n");
    let state = bar_state(&view, &mut cx);
    assert!(hint_labels(&state).is_empty(), "no hints beside the error token");
    assert!(state.error.is_some());
}

#[gpui::test]
fn overlay_hides_hints(cx: &mut TestAppContext) {
    let root = fixture("bar-hints-overlay");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    settle(&view, &mut cx);
    assert!(hint_labels(&bar_state(&view, &mut cx)).is_empty());
}

/// Install a running operation and hand back its cancel flag.
fn inject_running(cx: &mut VisualTestContext) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let handle = cancel.clone();
    cx.update(|_, cx| {
        cx.global_mut::<Operations>().running = Some(crate::operations::Running {
            label: "Move 2 items to Trash".into(),
            progress: crow_fs::ops::Progress { done_items: 1, total_items: 2, total_bytes: 0, ..Default::default() },
            cancel: handle,
            started: std::time::Instant::now(),
        });
        cx.refresh_windows();
    });
    cx.run_until_parked();
    cancel
}

#[gpui::test]
fn cancel_button_sets_the_cancel_flag(cx: &mut TestAppContext) {
    let root = fixture("bar-cancel-click");
    let (_view, mut cx) = open(cx, &root);
    let cancel = inject_running(&mut cx);
    let bounds = cx.debug_bounds("bar-cancel").expect("the Cancel button is laid out");
    let progress = cx.debug_bounds("bar-progress").expect("the progress line is laid out");
    assert!(progress.size.width > gpui::px(0.), "some progress is drawn");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    assert!(cancel.load(std::sync::atomic::Ordering::Relaxed), "the click asked the operation to stop");

    cx.update(|_, cx| cx.global_mut::<Operations>().running = None);
}

#[gpui::test]
fn escape_cancels_from_the_list(cx: &mut TestAppContext) {
    let root = fixture("bar-cancel-escape");
    let (_view, mut cx) = open(cx, &root);
    let cancel = inject_running(&mut cx);
    cx.simulate_keystrokes("escape");
    assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));

    cx.update(|_, cx| cx.global_mut::<Operations>().running = None);
}

#[gpui::test]
fn fuzz_generator_uses_the_live_keymap_and_screen(cx: &mut TestAppContext) {
    use crate::fuzz::{Step, StepGen, state};
    let root = fixture("fuzz-generator");
    let (view, mut cx) = open(cx, &root);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let state = state(&view, &mut cx);
    assert!(!state.all_keys.is_empty(), "the live keymap has bindings");
    assert!(state.all_keys.iter().any(|key| key == "cmd-k" || key == "space"), "{:?}", state.all_keys);
    assert!(state.selectors.iter().any(|selector| selector == "row-notes.txt"), "{:?}", state.selectors);
    assert!(!state.active_keys.is_empty(), "the list's context resolves bindings");

    let mut generator = StepGen::new(1);
    let steps: Vec<_> = (0..200).map(|_| generator.next(&state)).collect();
    assert!(steps.iter().any(|step| matches!(step, Step::Click(_))), "the generator clicks rows");
    assert!(steps.iter().any(|step| matches!(step, Step::Keys(_))), "and presses keys");
}

#[gpui::test]
fn config_error_token_opens_the_broken_file(cx: &mut TestAppContext) {
    let root = fixture("bar-error-open");
    let (view, mut cx) = open(cx, &root);
    effect_calls(&mut cx);
    write_config(&view, &mut cx, "sidebar.toml", "[[group]\n");

    let state = bar_state(&view, &mut cx);
    let error = state.error.expect("the broken sidebar shows a token");
    assert!(error.text.starts_with("sidebar.toml"), "{}", error.text);
    assert!(error.text.contains("line "), "the parser's line is shown: {}", error.text);
    let file = error.file.expect("a config token can be opened");
    assert!(file.ends_with("sidebar.toml"), "{}", file.display());

    let bounds = cx.debug_bounds("bar-error-open").expect("the Open button is laid out");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let opened: Vec<_> =
        effect_calls(&mut cx).into_iter().filter(|call| matches!(call, crate::effects::Call::OpenConfig(_))).collect();
    assert_eq!(opened, [crate::effects::Call::OpenConfig(file)]);
}

#[gpui::test]
fn file_effects_are_recorded_not_run(cx: &mut TestAppContext) {
    use crate::effects::Call;
    let root = fixture("effects-file");
    let (view, mut cx) = open(cx, &root);
    click_selector(&mut cx, "row-notes.txt");
    effect_calls(&mut cx);

    cx.simulate_keystrokes("cmd-down");
    let calls = effect_calls(&mut cx);
    assert!(calls.contains(&Call::OpenPath(root.join("notes.txt"))), "{calls:?}");

    cx.simulate_keystrokes("space");
    let calls = effect_calls(&mut cx);
    assert!(calls.contains(&Call::SetQuickLookKeyHandler), "{calls:?}");
    assert!(calls.contains(&Call::ToggleQuickLook(root.join("notes.txt"))), "{calls:?}");

    // Moving the cursor follows it in an open panel; the recorder sees that too.
    click_selector(&mut cx, "row-notes.txt");
    let calls = effect_calls(&mut cx);
    assert!(calls.contains(&Call::UpdateQuickLook(root.join("notes.txt"))), "{calls:?}");

    // Show in Finder is a menu/palette command, not a key.
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">show in finder");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    let calls = effect_calls(&mut cx);
    assert!(calls.contains(&Call::RevealInFinder(vec![root.join("notes.txt")])), "{calls:?}");
}

#[gpui::test]
fn copy_records_the_pasteboard_write(cx: &mut TestAppContext) {
    use crate::effects::Call;
    let root = fixture("effects-copy");
    let (_view, mut cx) = open(cx, &root);
    click_selector(&mut cx, "row-notes.txt");
    effect_calls(&mut cx);
    cx.simulate_keystrokes("cmd-c");
    let calls = effect_calls(&mut cx);
    assert!(calls.contains(&Call::WriteFileUrls(vec![root.join("notes.txt")])), "{calls:?}");
}

/// H4 batch 6 critique: above 1000 paths the system pasteboard write blocks the
/// main thread for seconds and 100k items wedge the pasteboard server. Crow's
/// own clipboard still holds every path, and the status says it is Crow only.
#[gpui::test]
fn a_huge_copy_keeps_the_clipboard_in_crow(cx: &mut TestAppContext) {
    use crate::effects::Call;
    let root = fixture("copy-cap");
    for i in 0..1001 {
        std::fs::write(root.join(format!("file-{i:04}.txt")), "").expect("create fixture");
    }
    let (_view, mut cx) = open(cx, &root);
    effect_calls(&mut cx);
    cx.simulate_keystrokes("cmd-a cmd-c");
    let calls = effect_calls(&mut cx);
    assert!(
        !calls.iter().any(|call| matches!(call, Call::WriteFileUrls(..))),
        "the system pasteboard is skipped: {calls:?}"
    );
    let clipboard = cx.read(|cx| cx.global::<Operations>().clipboard.as_ref().map(|clip| clip.paths.len()));
    // The fixture's alpha, beta and notes.txt plus the 1001 files above.
    assert_eq!(clipboard, Some(1004), "Crow's clipboard holds every path");
    let (paths, cut) = cx.read(crate::operations::pasteboard_paths);
    assert_eq!(paths.len(), 1004, "a paste uses Crow's clipboard");
    assert!(!cut, "a copy does not move");
    let message = cx.read(|cx| cx.global::<Operations>().message.clone());
    assert_eq!(message.map(|(text, _)| text.to_string()).as_deref(), Some("Copied 1004 items (Crow only)"));
}

#[gpui::test]
fn trash_records_its_sound(cx: &mut TestAppContext) {
    use crate::effects::Call;
    let root = fixture("effects-sound");
    let (view, mut cx) = open(cx, &root);
    click_selector(&mut cx, "row-notes.txt");
    effect_calls(&mut cx);
    cx.simulate_keystrokes("cmd-backspace");
    settle(&view, &mut cx);
    let calls = effect_calls(&mut cx);
    assert!(calls.iter().any(|call| matches!(call, Call::PlaySound(..))), "{calls:?}");
}

#[gpui::test]
fn context_menu_records_entries_and_runs_a_scripted_choice(cx: &mut TestAppContext) {
    use crate::context_menu::{MenuKind, slots};
    use crate::effects::Call;
    use crate::palette::CommandId;
    let root = fixture("effects-menu");
    let (view, mut cx) = open(cx, &root);
    click_selector(&mut cx, "row-notes.txt");
    let ix = slots(MenuKind::Items)
        .iter()
        .position(|slot| *slot == Some(CommandId::MoveToTrash))
        .expect("the trash line is in the menu");
    cx.update(|_, cx| {
        crate::effects::recording(cx).expect("recording effects").set_context_menu_choice(Some(ix));
    });
    effect_calls(&mut cx);
    let at = row_center(&mut cx, "row-notes.txt");
    right_click(&mut cx, at);
    cx.run_until_parked();
    settle(&view, &mut cx);
    assert!(!root.join("notes.txt").exists(), "the scripted menu choice ran");
    let calls = effect_calls(&mut cx);
    assert!(
        calls.iter().any(|call| matches!(call, Call::ContextMenu(entries) if entries.iter().any(|entry| {
            matches!(entry, crow_macos::ContextMenuEntry::Item { title, .. } if title == "Move to Trash")
        }))),
        "{calls:?}"
    );
    cx.update(|_, cx| crate::effects::recording(cx).expect("recording effects").clear_context_menu_choice());
}

#[gpui::test]
fn why_sheet_edits_views_toml_through_effects(cx: &mut TestAppContext) {
    use crate::effects::Call;
    let root = fixture("effects-why");
    let (view, mut cx) = open(cx, &root);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    cx.simulate_keystrokes("cmd-j");
    assert!(view.read_with(&cx, |w, _| w.why.is_some()), "cmd-j opens Why");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    click_selector(&mut cx, "why-open-file");
    let calls = effect_calls(&mut cx);
    assert!(calls.contains(&Call::OpenText(config_dir(&cx).join("views.toml"))), "{calls:?}");
}

#[gpui::test]
fn fixing_the_file_clears_the_token(cx: &mut TestAppContext) {
    let root = fixture("bar-error-fixed");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", "[[group]\n");
    assert!(bar_state(&view, &mut cx).error.is_some());

    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[]));
    let state = bar_state(&view, &mut cx);
    assert!(state.error.is_none(), "fixing the file removes the token");
    assert_eq!(hint_labels(&state), ["filter"], "hints come back");
}

#[gpui::test]
fn pane_error_has_no_open_button(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let root = fixture("bar-error-pane");
    let locked = root.join("locked");
    std::fs::create_dir_all(&locked).expect("create folder");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("lock folder");
    let (view, mut cx) = open(cx, &root);
    pane(&view, &cx).update(&mut cx, |pane, cx| pane.navigate(locked.clone(), cx));
    settle(&view, &mut cx);

    let state = bar_state(&view, &mut cx);
    let error = state.error.expect("the pane's error shows");
    assert!(error.file.is_none(), "a folder error has nothing to open");
    assert!(cx.debug_bounds("bar-error-open").is_none());

    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).expect("unlock folder");
}

#[gpui::test]
fn narrow_window_drops_hints_before_the_path(cx: &mut TestAppContext) {
    let root = fixture("bar-narrow");
    let (view, mut cx) = open(cx, &root);
    let last_segment =
        |state: &BarState| -> &'static str { format!("path-segment-{}", state.path.parts.len() - 1).leak() };

    // Shrink step by step: the middle folds first, then the hint goes; the first and
    // last segments stay on screen throughout.
    let (mut folded_with_hint, mut hint_gone) = (false, false);
    for width in (380..=1200).rev().step_by(20) {
        cx.simulate_resize(gpui::size(px(width as f32), px(500.)));
        cx.run_until_parked();
        let state = bar_state(&view, &mut cx);
        let hints = hint_labels(&state);
        folded_with_hint |= state.path.hidden.is_some() && !hints.is_empty();
        if hints.is_empty() {
            hint_gone = true;
            assert!(state.path.hidden.is_some(), "at {width}: the hint only goes once the path has folded");
        } else {
            assert!(!hint_gone, "at {width}: a dropped hint stays dropped while narrowing");
        }
        assert!(cx.debug_bounds("path-segment-0").is_some(), "at {width}: the first segment stays");
        let tail = cx.debug_bounds(last_segment(&state)).expect("the last segment is drawn");
        assert!(tail.right() <= px(width as f32), "at {width}: the last segment is on screen: {tail:?}");
    }
    assert!(folded_with_hint, "the path folds before the hint is dropped");
    assert!(hint_gone, "a narrow window drops the hint");
    assert!(cx.debug_bounds("path-folded").is_some(), "the fold is drawn as …");

    cx.simulate_resize(gpui::size(px(3000.), px(800.)));
    cx.run_until_parked();
    let state = bar_state(&view, &mut cx);
    assert_eq!(hint_labels(&state), ["filter"], "wide again shows the hint");
    assert_eq!(state.path.hidden, None, "and the whole path");
}

#[gpui::test]
fn error_and_progress_leave_the_path_its_ends(cx: &mut TestAppContext) {
    let root = fixture("bar-squeeze");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_resize(gpui::size(px(700.), px(500.)));
    cx.update(|_, cx| {
        cx.global_mut::<Operations>().running = Some(crate::operations::Running {
            label: "Copy 12 items to a folder with a rather long name indeed".into(),
            progress: crow_fs::ops::Progress { done_bytes: 1, total_bytes: 4, ..Default::default() },
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            started: std::time::Instant::now(),
        });
    });
    let dir = config_dir(&cx);
    std::fs::write(dir.join("views.toml"), "[defaults]\nsort = \"a colour that does not exist at all\"\n")
        .expect("break views.toml");
    cx.update(|_, cx| crate::ui::reload_config(cx));
    cx.run_until_parked();
    let state = bar_state(&view, &mut cx);
    let max = state.transient_max.expect("the transient slot is capped");
    assert!(max < px(700.), "{max:?}");
    let last: &'static str = format!("path-segment-{}", state.path.parts.len() - 1).leak();
    let first = cx.debug_bounds("path-segment-0").expect("the first segment is drawn");
    let tail = cx.debug_bounds(last).expect("the last segment is drawn");
    assert!(first.size.width > px(20.), "the first segment keeps its room: {first:?}");
    assert!(tail.size.width > px(20.), "the last segment keeps its room: {tail:?}");
    let counts = cx.debug_bounds("bar-cancel").expect("Cancel is drawn");
    assert!(counts.right() <= px(700.), "the transient stays inside the window: {counts:?}");
    cx.update(|_, cx| cx.global_mut::<Operations>().running = None);
}

#[gpui::test]
fn counts_name_the_focused_folder(cx: &mut TestAppContext) {
    let root = fixture("bar-counts-columns");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3 right");
    settle(&view, &mut cx);
    let state = bar_state(&view, &mut cx);
    assert_eq!(state.count.text.as_ref(), "1 item");
    assert_eq!(state.count.spoken.as_ref(), "1 item in alpha", "the focused column's folder");
}

#[gpui::test]
fn counts_show_filter_and_selection(cx: &mut TestAppContext) {
    let root = fixture("bar-counts-filter");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("/");
    cx.simulate_input("no");
    settle(&view, &mut cx);
    let state = bar_state(&view, &mut cx);
    assert_eq!(state.count.text.as_ref(), "3 items", "the field says \"1 of 3\"; the bar keeps the folder's count");

    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("cmd-a");
    settle(&view, &mut cx);
    let state = bar_state(&view, &mut cx);
    assert_eq!(state.count.text.as_ref(), "3 selected · 6 bytes", "folders add no bytes");
    let folder = crate::style::display_name(&root);
    assert_eq!(state.count.spoken.as_ref(), format!("3 of 3 selected in {folder}, 6 bytes"));
}

#[gpui::test]
fn one_item_is_singular(cx: &mut TestAppContext) {
    let root = fixture("bar-singular");
    std::fs::create_dir_all(root.join("solo")).expect("create folder");
    std::fs::write(root.join("solo/only.txt"), "").expect("create file");
    let (view, mut cx) = open(cx, &root.join("solo"));
    let state = bar_state(&view, &mut cx);
    assert_eq!(state.count.text.as_ref(), "1 item");
    assert_eq!(state.count.spoken.as_ref(), "1 item in solo");
}

#[gpui::test]
fn message_does_not_hide_the_count(cx: &mut TestAppContext) {
    let root = fixture("bar-message");
    let (view, mut cx) = open(cx, &root);
    cx.update(|_, cx| {
        operations::message("Copied 1 item", false, cx);
    });
    cx.simulate_keystrokes("down");
    let state = bar_state(&view, &mut cx);
    assert_eq!(primary_text(&state), "Copied 1 item");
    assert_eq!(state.count.text.as_ref(), "3 items");
}

#[gpui::test]
fn info_messages_expire_but_errors_stay(cx: &mut TestAppContext) {
    let root = fixture("bar-message-expire");
    let (_view, mut cx) = open(cx, &root);
    cx.update(|_, cx| operations::message("Copied 1 item", false, cx));
    let executor = cx.executor();
    executor.advance_clock(std::time::Duration::from_secs(6));
    cx.run_until_parked();
    assert!(cx.read(|cx| cx.global::<Operations>().message.is_none()), "an info message retires");

    cx.update(|_, cx| operations::message("Can't open x", true, cx));
    executor.advance_clock(std::time::Duration::from_secs(6));
    cx.run_until_parked();
    assert!(cx.read(|cx| cx.global::<Operations>().message.is_some()), "an error waits for something else");
}

#[gpui::test]
fn copy_and_missing_mark_messages_retire(cx: &mut TestAppContext) {
    let root = fixture("bar-message-copy");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-c");
    settle(&view, &mut cx);
    let state = bar_state(&view, &mut cx);
    assert!(primary_text(&state).starts_with("Copied"), "{}", primary_text(&state));
    let executor = cx.executor();
    executor.advance_clock(std::time::Duration::from_secs(6));
    cx.run_until_parked();
    assert!(cx.read(|cx| cx.global::<Operations>().message.is_none()), "the copy message retires");

    use_vim(&view, &mut cx);
    cx.simulate_keystrokes("' q");
    settle(&view, &mut cx);
    let state = bar_state(&view, &mut cx);
    assert_eq!(primary_text(&state), "No mark q");
    assert!(state.error.is_none(), "a missing mark is not an error token");
    executor.advance_clock(std::time::Duration::from_secs(6));
    cx.run_until_parked();
    assert!(cx.read(|cx| cx.global::<Operations>().message.is_none()), "and it retires too");
}

#[gpui::test]
fn option_arrows_move_by_word_in_text_fields(cx: &mut TestAppContext) {
    let root = fixture("text-word-moves");
    let (view, mut cx) = open(cx, &root);
    let filter = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, cx| p.filter_text(cx)).unwrap_or_default();
    cx.simulate_keystrokes("/");
    cx.simulate_input("m5-fix.md");
    cx.simulate_keystrokes("alt-left alt-left");
    cx.simulate_input("X");
    assert_eq!(filter(&cx), "m5-Xfix.md", "⌥← twice lands before \"fix\"");

    cx.simulate_keystrokes("alt-right");
    cx.simulate_input("Y");
    assert_eq!(filter(&cx), "m5-XfixY.md", "⌥→ goes to the end of the word");

    cx.simulate_keystrokes("alt-backspace");
    assert_eq!(filter(&cx), "m5-.md", "⌥⌫ deletes back to the word start");

    cx.simulate_keystrokes("cmd-left alt-shift-right");
    cx.simulate_input("z");
    assert_eq!(filter(&cx), "z-.md", "⌥⇧→ selects the word");
}

#[gpui::test]
fn caret_blinks_in_text_fields(cx: &mut TestAppContext) {
    let root = fixture("caret-blink");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    settle(&view, &mut cx);
    let input = view.read_with(&cx, |w, _| w.palette.as_ref().expect("the palette is open").input.clone());
    assert!(input.read_with(&cx, |input, _| input.caret_visible()), "visible while the field has focus");

    let executor = cx.executor();
    executor.advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    assert!(!input.read_with(&cx, |input, _| input.caret_visible()), "the caret goes off after 500 ms");
    executor.advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    assert!(input.read_with(&cx, |input, _| input.caret_visible()), "and back on after another 500 ms");

    cx.simulate_input("a");
    assert!(input.read_with(&cx, |input, _| input.caret_visible()), "typing shows the caret at once");

    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    executor.advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    assert!(!input.read_with(&cx, |input, _| input.caret_visible()), "no caret without focus");
}

#[gpui::test]
fn navigating_clears_an_operation_error(cx: &mut TestAppContext) {
    let root = fixture("bar-message-nav");
    let (view, mut cx) = open(cx, &root);
    cx.update(|_, cx| operations::message("Can't open x", true, cx));
    cx.simulate_keystrokes("cmd-down");
    settle(&view, &mut cx);
    assert!(cx.read(|cx| cx.global::<Operations>().message.is_none()), "navigation drops the old message");
}

#[gpui::test]
fn error_and_running_operation_show_together(cx: &mut TestAppContext) {
    let root = fixture("bar-error-running");
    let (view, mut cx) = open(cx, &root);
    cx.update(|_, cx| {
        cx.global_mut::<Operations>().running = Some(crate::operations::Running {
            label: "Copy 1 item to x".into(),
            progress: crow_fs::ops::Progress { total_items: 1, total_bytes: 1, ..Default::default() },
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            started: std::time::Instant::now(),
        });
    });
    let dir = config_dir(&cx);
    std::fs::write(dir.join("views.toml"), "[defaults]\nsort = \"colour\"\n").expect("break views.toml");
    cx.update(|_, cx| crate::ui::reload_config(cx));
    cx.run_until_parked();

    let state = bar_state(&view, &mut cx);
    assert!(matches!(state.primary, Primary::Running(_)), "progress stays primary");
    let error = state.error.expect("the config error is its own token");
    assert!(error.file.as_ref().is_some_and(|f| f.ends_with("views.toml")));
    assert!(error.text.starts_with("views.toml"), "{}", error.text);

    cx.update(|_, cx| cx.global_mut::<Operations>().running = None);
    settle(&view, &mut cx);
}

/// The in-memory count for an action name.
fn key_uses(cx: &VisualTestContext, key: &str) -> u32 {
    cx.read(|cx| crate::usage::count_key(key, cx))
}

/// Wait for the background save and the count it wrote to `usage.json`.
fn saved_uses(cx: &mut VisualTestContext, key: &str, expected: u32) -> u32 {
    let path = cx.read(|cx| cx.global::<Ui>().state_dir.join(crow_core::USAGE_FILE));
    let mut saved = 0;
    for _ in 0..200 {
        cx.run_until_parked();
        saved = crow_core::Usage::load(&path).count(key);
        if saved == expected {
            return saved;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    saved
}

#[gpui::test]
fn pressing_slash_counts_filter_uses(cx: &mut TestAppContext) {
    let root = fixture("usage-filter");
    let (view, mut cx) = open(cx, &root);
    for _ in 0..3 {
        cx.simulate_keystrokes("/");
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
    }
    let key = StartFilter.name();
    assert_eq!(key_uses(&cx, key), 3, "each / counted once");
    assert_eq!(saved_uses(&mut cx, key, 3), 3, "and survived to disk");
}

#[gpui::test]
fn palette_and_dispatch_count_too(cx: &mut TestAppContext) {
    let root = fixture("usage-palette");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">filter");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert!(pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()), "the palette opened the filter");
    cx.simulate_keystrokes("escape");

    cx.dispatch_action(ShowSearch);
    settle(&view, &mut cx);
    cx.simulate_keystrokes("escape");

    assert_eq!(key_uses(&cx, ShowCommandPalette.name()), 1, "Cmd-K counted");
    assert_eq!(key_uses(&cx, StartFilter.name()), 1, "the palette run counted");
    assert_eq!(key_uses(&cx, ShowSearch.name()), 1, "a dispatched action counted");
}

#[gpui::test]
fn refocusing_an_open_filter_does_not_count(cx: &mut TestAppContext) {
    let root = fixture("usage-refocus");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("/");
    settle(&view, &mut cx);
    let before = key_uses(&cx, StartFilter.name());
    assert_eq!(before, 1);
    cx.dispatch_action(StartFilter);
    settle(&view, &mut cx);
    assert_eq!(key_uses(&cx, StartFilter.name()), before, "focusing the open field is not a new use");
}

#[gpui::test]
fn rules_apply_live_and_why_explains(cx: &mut TestAppContext) {
    let root = fixture("rules");
    let (view, mut cx) = open(cx, &root);
    let sort = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, _| p.sort());
    assert_eq!(sort(&cx).key, crow_fs::SortKey::Name);

    let rules = format!("{}\n[[rule]]\npath = \"{}\"\nsort = \"size\"\n", crow_core::DEFAULT_VIEWS, root.display());
    write_config(&view, &mut cx, "views.toml", &rules);
    assert_eq!(sort(&cx).key, crow_fs::SortKey::Size, "an edited views.toml applies to the open folder");
    assert!(!sort(&cx).ascending, "sizes start largest first");

    cx.simulate_keystrokes("cmd-j");
    assert!(view.read_with(&cx, |w, _| w.why.is_some()), "Cmd-J asks why");
    let spoken = cx.update(|window, cx| {
        let _ = window;
        let w = view.read(cx);
        let ui = cx.global::<Ui>();
        ui.views.resolve(&w.pane().read(cx).current_dir(cx)).sort.1.to_string()
    });
    assert_eq!(spoken, format!("rule \"{}\" in views.toml", root.display()));
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(&cx, |w, _| w.why.is_none()));

    // A broken file keeps the last good settings and says why.
    write_config(&view, &mut cx, "views.toml", "[defaults]\nsort = \"colour\"\n");
    assert_eq!(sort(&cx).key, crow_fs::SortKey::Size);
    let error = cx.update(|_, cx| cx.global::<Ui>().config_error());
    assert!(error.is_some_and(|e| e.contains("colour")));
}

fn use_vim(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) {
    write_config(view, cx, "settings.toml", "keymap = \"vim\"\n");
}

#[gpui::test]
fn vim_layer_moves_opens_and_goes_up(cx: &mut TestAppContext) {
    let root = fixture("vim");
    let (view, mut cx) = open(cx, &root);
    use_vim(&view, &mut cx);
    cx.simulate_keystrokes("j");
    assert_eq!(selected(&view, &cx).as_deref(), Some("beta"));
    cx.simulate_keystrokes("shift-g");
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"));
    cx.simulate_keystrokes("g g");
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"));
    cx.simulate_keystrokes("n");
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "letters don't select by name");
    cx.simulate_keystrokes("l");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root, "l expands the folder first");
    cx.simulate_keystrokes("l");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"), "l again goes into it");
    cx.simulate_keystrokes("h");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root);
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "h selects where you came from");
}

#[gpui::test]
fn vim_marks_set_favorites_and_jump(cx: &mut TestAppContext) {
    let root = fixture("marks");
    let (view, mut cx) = open(cx, &root.join("alpha"));
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[]));
    use_vim(&view, &mut cx);
    cx.simulate_keystrokes("m");
    assert!(pane(&view, &cx).read_with(&cx, |p, _| p.pending_hint().is_some()), "m waits for a letter");
    cx.simulate_keystrokes("j");
    settle(&view, &mut cx);
    assert_eq!(sidebar_paths(&cx), [root.join("alpha")], "the folder became a favorite with mark j");
    let text = std::fs::read_to_string(config_dir(&cx).join("sidebar.toml")).unwrap();
    assert!(text.contains("key = \"j\""), "{text}");

    cx.simulate_keystrokes("h");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root);
    cx.simulate_keystrokes("' j");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"), "'j jumps to the mark");
    cx.simulate_keystrokes("' q");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"), "an unknown mark goes nowhere");
}

/// Two themes in a temp theme index, shared by every test (the variable is process-wide).
fn theme_index() -> &'static Path {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("crow-app-theme-index-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("schemes")).unwrap();
        let base = include_str!("../../../crow-theme/src/fallback.json");
        std::fs::write(dir.join("schemes/firebox.json"), base).unwrap();
        let light = base.replace("\"dark\"", "\"light\"").replace("FireBox", "Paper").replace("#111111", "#fafafa");
        std::fs::write(dir.join("schemes/paper.json"), light).unwrap();
        std::fs::write(dir.join("defaults.json"), r#"{"dark":"firebox","light":"paper"}"#).unwrap();
        // SAFETY: set once, before any test reads it; all tests see the same value.
        unsafe {
            std::env::set_var("THEMES_DIR", &dir);
            std::env::set_var("THEMES_STATE_DIR", dir.join("state"));
        }
        dir
    })
}

fn wait_for_search(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) {
    for _ in 0..400 {
        cx.run_until_parked();
        let busy = view.read_with(cx, |w, cx| w.search.as_ref().is_some_and(|search| search.read(cx).is_searching()));
        if !busy {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("search never finished");
}

/// Wait until the search preview for `path` has loaded.
fn wait_for_preview(view: &Entity<CrowWindow>, cx: &mut VisualTestContext, path: &Path) {
    for _ in 0..400 {
        cx.run_until_parked();
        let loaded = view.read_with(cx, |w, cx| w.search.as_ref().and_then(|search| search.read(cx).preview_path()));
        if loaded.as_deref() == Some(path) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("preview for {} never loaded", path.display());
}

/// Click the element with this debug selector, failing if it is not on screen.
fn click_selector(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} is on screen"));
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
}

#[gpui::test]
fn search_panel_streams_names_and_contents(cx: &mut TestAppContext) {
    let root = fixture("search");
    std::fs::write(root.join("notes.txt"), "needle here\n").expect("create fixture");
    std::fs::write(root.join("alpha/needle.txt"), "nothing\n").expect("create fixture");
    std::fs::write(root.join("beta/other.txt"), "needle in beta\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);

    cx.simulate_keystrokes("cmd-f");
    settle(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    assert_eq!(search.read_with(&cx, |search, _| search.scope()), crate::search::Scope::Folder);
    cx.simulate_input("needle");
    wait_for_search(&view, &mut cx);

    let paths = search.read_with(&cx, |search, _| search.result_paths());
    assert!(paths.contains(&root.join("notes.txt")), "{paths:?}");
    assert!(paths.contains(&root.join("alpha/needle.txt")), "{paths:?}");
    assert!(paths.contains(&root.join("beta/other.txt")), "contents are searched too: {paths:?}");
    assert_eq!(
        search.read_with(&cx, |search, _| search.result_content(&root.join("beta/other.txt"))),
        Some((1, "needle in beta".to_owned()))
    );
}

#[gpui::test]
fn search_scope_buttons_switch_scope(cx: &mut TestAppContext) {
    let root = fixture("search-scope");
    std::fs::write(root.join("notes.txt"), "needle\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    settle(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");

    // Empty query: switching scope starts no walk, Home or Spotlight.
    click_selector(&mut cx, "search-scope-home");
    settle(&view, &mut cx);
    assert_eq!(search.read_with(&cx, |search, _| search.scope()), crate::search::Scope::Home);

    click_selector(&mut cx, "search-scope-this-mac");
    settle(&view, &mut cx);
    assert_eq!(search.read_with(&cx, |search, _| search.scope()), crate::search::Scope::ThisMac);

    click_selector(&mut cx, "search-scope-folder");
    settle(&view, &mut cx);
    assert_eq!(search.read_with(&cx, |search, _| search.scope()), crate::search::Scope::Folder);

    cx.simulate_input("notes");
    wait_for_search(&view, &mut cx);
    let paths = search.read_with(&cx, |search, _| search.result_paths());
    assert!(paths.contains(&root.join("notes.txt")), "{paths:?}");
}

#[gpui::test]
fn search_everything_turns_the_rules_off(cx: &mut TestAppContext) {
    let root = fixture("search-rules");
    std::fs::create_dir_all(root.join("node_modules/pkg")).expect("create fixture");
    std::fs::write(root.join("node_modules/pkg/needle.txt"), "needle\n").expect("create fixture");
    std::fs::write(root.join("outside.txt"), "needle\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);

    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("needle");
    wait_for_search(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    let paths = search.read_with(&cx, |search, _| search.result_paths());
    assert!(paths.contains(&root.join("outside.txt")));
    assert!(!paths.iter().any(|path| path.ends_with("node_modules/pkg/needle.txt")), "{paths:?}");
    assert!(search.read_with(&cx, |search, _| search.skipped_count()) > 0);

    cx.simulate_keystrokes("cmd-shift-a");
    wait_for_search(&view, &mut cx);
    assert!(search.read_with(&cx, |search, _| search.everything()));
    let paths = search.read_with(&cx, |search, _| search.result_paths());
    assert!(paths.contains(&root.join("node_modules/pkg/needle.txt")), "{paths:?}");
    assert_eq!(search.read_with(&cx, |search, _| search.skipped_count()), 0);
}

#[gpui::test]
fn search_row_aria_reads_line(cx: &mut TestAppContext) {
    let root = fixture("search-aria");
    std::fs::write(root.join("other.txt"), "needle in beta\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("needle");
    wait_for_search(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    let paths = search.read_with(&cx, |search, _| search.result_paths());
    let index = paths.iter().position(|path| path.ends_with("other.txt")).expect("content hit");
    let label = search.read_with(&cx, |search, _| search.row_label(index)).expect("a row");
    assert!(label.starts_with("other.txt, in "), "{label}");
    assert!(label.ends_with(", line 1: needle in beta"), "{label}");
}

#[gpui::test]
fn search_preview_follows_the_selection(cx: &mut TestAppContext) {
    let root = fixture("search-preview");
    std::fs::write(root.join("a-notes.txt"), "alpha\n").expect("create fixture");
    std::fs::write(root.join("b-notes.txt"), "beta\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("notes");
    wait_for_search(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    let paths = search.read_with(&cx, |search, _| search.result_paths());
    assert!(paths.len() >= 2, "{paths:?}");
    wait_for_preview(&view, &mut cx, &paths[0]);

    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("down");
    wait_for_preview(&view, &mut cx, &paths[1]);
    assert_eq!(search.read_with(&cx, |search, _| search.cursor()), 1);
}

#[gpui::test]
fn search_preview_shows_in_column_view(cx: &mut TestAppContext) {
    let root = fixture("search-preview-columns");
    std::fs::write(root.join("notes.txt"), "hello\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("notes");
    wait_for_search(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    let path = search.read_with(&cx, |search, _| search.result_paths()).first().cloned().expect("a result");
    wait_for_preview(&view, &mut cx, &path);
    settle(&view, &mut cx);
    assert!(cx.debug_bounds("preview").is_some(), "the preview pane is on screen");
}

#[gpui::test]
fn search_preview_shows_the_hit_line(cx: &mut TestAppContext) {
    let root = fixture("search-preview-hit");
    let mut body = String::new();
    for n in 1..=100 {
        body.push_str(if n == 80 { "needle on line 80\n" } else { "filler\n" });
    }
    std::fs::write(root.join("deep.txt"), body).expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("needle");
    wait_for_search(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    wait_for_preview(&view, &mut cx, &root.join("deep.txt"));
    let (hit, text) = search.read_with(&cx, |search, _| search.preview_excerpt()).expect("an excerpt");
    assert_eq!(hit, 80);
    assert!(text.lines().any(|line| line == "needle on line 80"), "{text}");
}

#[gpui::test]
fn search_results_vim_j_k_move_the_cursor(cx: &mut TestAppContext) {
    let root = fixture("search-vim-results");
    std::fs::write(root.join("notes-a.txt"), "alpha\n").expect("create fixture");
    std::fs::write(root.join("notes-b.txt"), "beta\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    use_vim(&view, &mut cx);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("notes");
    wait_for_search(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    assert!(search.read_with(&cx, |search, _| search.result_paths()).len() >= 2);
    cx.simulate_keystrokes("tab");
    assert_eq!(search.read_with(&cx, |search, _| search.cursor()), 0);
    cx.simulate_keystrokes("j");
    assert_eq!(search.read_with(&cx, |search, _| search.cursor()), 1);
    cx.simulate_keystrokes("k");
    assert_eq!(search.read_with(&cx, |search, _| search.cursor()), 0);
}

#[gpui::test]
fn search_name_hits_outrank_content_hits(cx: &mut TestAppContext) {
    let root = fixture("search-order");
    std::fs::write(root.join("m5-notes.md"), "nothing\n").expect("create fixture");
    std::fs::write(root.join("deep.txt"), "m5 lives here\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("m5");
    wait_for_search(&view, &mut cx);
    let paths = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search").read(cx).result_paths());
    assert_eq!(paths.first().and_then(|path| path.file_name()), Some("m5-notes.md".as_ref()), "{paths:?}");
    assert!(paths.iter().any(|path| path.ends_with("deep.txt")), "{paths:?}");
}

#[gpui::test]
fn search_selects_the_best_first_row_until_the_user_moves(cx: &mut TestAppContext) {
    let root = fixture("search-selection");
    // Name hits live outside the walked root, so the real "qqzz" search finds nothing.
    let outside = root.parent().expect("temp parent").join(format!("crow-qqzz-{}", std::process::id()));
    std::fs::create_dir_all(&outside).expect("create fixture");
    let name_path = outside.join("qqzz-zebra.txt");
    std::fs::write(&name_path, "nothing\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    settle(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    cx.simulate_input("qqzz");
    wait_for_search(&view, &mut cx);
    let hit = |path: PathBuf, kind: HitKind, score| Hit { path, is_dir: false, score, kind, modified: None, size: 0 };

    // The walker found a content hit first; the name hit arrives in a later batch.
    let content = hit(
        root.join("notes.txt"),
        HitKind::Content { line: 1, text: "qqzz lives here".to_owned(), ranges: test_ranges(0, 4), more: 0 },
        0,
    );
    search.update(&mut cx, |search, cx| search.add_test_hits(vec![content], cx));
    assert_eq!(
        search.read_with(&cx, |search, _| search.selected_entry()).map(|(path, _)| path),
        Some(root.join("notes.txt"))
    );

    let name = hit(name_path.clone(), HitKind::Name { ranges: test_ranges(0, 4) }, 100);
    search.update(&mut cx, |search, cx| search.add_test_hits(vec![name], cx));
    wait_for_preview(&view, &mut cx, &name_path);
    assert_eq!(search.read_with(&cx, |search, _| search.cursor()), 0);
    assert_eq!(search.read_with(&cx, |search, _| search.selected_entry()).map(|(path, _)| path), Some(name_path));
}

#[gpui::test]
fn search_keeps_the_users_row_when_better_rows_arrive(cx: &mut TestAppContext) {
    let root = fixture("search-selection-moved");
    let outside = root.parent().expect("temp parent").join(format!("crow-qqzz-moved-{}", std::process::id()));
    std::fs::create_dir_all(&outside).expect("create fixture");
    let first_path = outside.join("qqzz-zebra.txt");
    let better_path = outside.join("qqzz-better.txt");
    std::fs::write(&first_path, "nothing\n").expect("create fixture");
    std::fs::write(&better_path, "nothing\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    settle(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    cx.simulate_input("qqzz");
    wait_for_search(&view, &mut cx);
    let hit = |path: PathBuf, kind: HitKind, score| Hit { path, is_dir: false, score, kind, modified: None, size: 0 };
    let first = hit(first_path, HitKind::Name { ranges: test_ranges(0, 4) }, 100);
    let content = hit(
        root.join("notes.txt"),
        HitKind::Content { line: 1, text: "qqzz lives here".to_owned(), ranges: test_ranges(0, 4), more: 0 },
        0,
    );
    search.update(&mut cx, |search, cx| search.add_test_hits(vec![content, first], cx));
    settle(&view, &mut cx);

    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("down");
    assert_eq!(search.read_with(&cx, |search, _| search.cursor()), 1);
    assert_eq!(
        search.read_with(&cx, |search, _| search.selected_entry()).map(|(path, _)| path),
        Some(root.join("notes.txt"))
    );

    let better = hit(better_path, HitKind::Name { ranges: test_ranges(0, 4) }, 200);
    search.update(&mut cx, |search, cx| search.add_test_hits(vec![better], cx));
    assert_eq!(search.read_with(&cx, |search, _| search.cursor()), 2, "the user's row moved down");
    assert_eq!(
        search.read_with(&cx, |search, _| search.selected_entry()).map(|(path, _)| path),
        Some(root.join("notes.txt"))
    );
}

#[gpui::test]
fn search_contiguous_name_hits_come_before_contents_and_weak_names(cx: &mut TestAppContext) {
    let root = fixture("search-weak-names");
    let outside = root.parent().expect("temp parent").join(format!("crow-qqzz-order-{}", std::process::id()));
    std::fs::create_dir_all(&outside).expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    settle(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    cx.simulate_input("qqzz");
    wait_for_search(&view, &mut cx);
    let strong = outside.join("qqzz-zebra.txt");
    let weak = outside.join("q-a-q-b-z-c-z.txt");
    let content = root.join("notes.txt");
    let hit = |path: PathBuf, kind: HitKind, score| Hit { path, is_dir: false, score, kind, modified: None, size: 0 };
    search.update(&mut cx, |search, cx| {
        search.add_test_hits(
            vec![
                hit(
                    content.clone(),
                    HitKind::Content {
                        line: 1,
                        text: "qqzz lives here".to_owned(),
                        ranges: test_ranges(0, 4),
                        more: 0,
                    },
                    0,
                ),
                hit(weak.clone(), HitKind::Name { ranges: vec![0..1, 2..3, 4..6, 7..8] }, 80),
                hit(strong.clone(), HitKind::Name { ranges: test_ranges(0, 4) }, 100),
            ],
            cx,
        );
    });
    let paths = search.read_with(&cx, |search, _| search.result_paths());
    assert_eq!(paths, [strong, content, weak]);
}

#[gpui::test]
fn search_footer_counts_hits_and_files(cx: &mut TestAppContext) {
    let root = fixture("search-footer");
    std::fs::write(root.join("a.txt"), "needle\nx\nneedle\nneedle\n").expect("create fixture");
    std::fs::write(root.join("b.txt"), "needle\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("needle");
    wait_for_search(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    assert_eq!(search.read_with(&cx, |search, _| search.hit_count()), 4);
    assert_eq!(search.read_with(&cx, |search, _| search.result_paths().len()), 2);
    assert_eq!(search.read_with(&cx, |search, _| search.footer_summary_text()), "4 results in 2 files");
    assert!(cx.debug_bounds("search-footer").is_some());
}

#[gpui::test]
fn search_skipped_link_opens_the_list_and_escape_closes_it_first(cx: &mut TestAppContext) {
    let root = fixture("search-skipped");
    std::fs::create_dir_all(root.join("node_modules/pkg")).expect("create fixture");
    std::fs::write(root.join("node_modules/pkg/needle.txt"), "needle\n").expect("create fixture");
    std::fs::write(root.join("outside.txt"), "needle\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("needle");
    wait_for_search(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    assert!(search.read_with(&cx, |search, _| search.skipped_count()) > 0);

    click_selector(&mut cx, "search-skipped-link");
    settle(&view, &mut cx);
    assert!(search.read_with(&cx, |search, _| search.skipped_open()));
    assert!(cx.debug_bounds("search-skipped-row-0").is_some());

    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    assert!(view.read_with(&cx, |w, _| w.search.is_some()), "Esc closed the list, not search");
    assert!(!search.read_with(&cx, |search, _| search.skipped_open()));

    click_selector(&mut cx, "search-skipped-link");
    settle(&view, &mut cx);
    click_selector(&mut cx, "search-skipped-close");
    settle(&view, &mut cx);
    assert!(!search.read_with(&cx, |search, _| search.skipped_open()));
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    assert!(view.read_with(&cx, |w, _| w.search.is_none()), "the next Esc closes search");
}

#[gpui::test]
fn search_invalid_regex_shows_the_error(cx: &mut TestAppContext) {
    let root = fixture("search-error");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("/(/");
    wait_for_search(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    let error = search.read_with(&cx, |search, _| search.error_text()).expect("an error");
    assert_eq!(error, "Invalid pattern: unclosed group");
    assert!(!error.contains('\n'), "{error}");
    assert!(search.read_with(&cx, |search, _| search.result_paths().is_empty()));
    let footer = search.read_with(&cx, |search, _| search.footer_summary_text());
    assert_eq!(footer, "Invalid pattern: unclosed group");
}

#[gpui::test]
fn search_switch_flips_rules(cx: &mut TestAppContext) {
    let root = fixture("search-switch");
    std::fs::create_dir_all(root.join("node_modules/pkg")).expect("create fixture");
    std::fs::write(root.join("node_modules/pkg/needle.txt"), "needle\n").expect("create fixture");
    std::fs::write(root.join("outside.txt"), "needle\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("needle");
    wait_for_search(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    assert!(!search.read_with(&cx, |search, _| search.everything()));

    click_selector(&mut cx, "search-skip-switch");
    wait_for_search(&view, &mut cx);
    assert!(search.read_with(&cx, |search, _| search.everything()));
    let paths = search.read_with(&cx, |search, _| search.result_paths());
    assert!(paths.contains(&root.join("node_modules/pkg/needle.txt")), "{paths:?}");

    click_selector(&mut cx, "search-skip-switch");
    wait_for_search(&view, &mut cx);
    assert!(!search.read_with(&cx, |search, _| search.everything()));
    let paths = search.read_with(&cx, |search, _| search.result_paths());
    assert!(!paths.iter().any(|path| path.ends_with("node_modules/pkg/needle.txt")), "{paths:?}");
}

#[gpui::test]
fn search_switch_hidden_for_this_mac(cx: &mut TestAppContext) {
    let root = fixture("search-switch-spotlight");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    settle(&view, &mut cx);
    assert!(cx.debug_bounds("search-skip-switch").is_some());
    click_selector(&mut cx, "search-scope-this-mac");
    settle(&view, &mut cx);
    assert!(cx.debug_bounds("search-skip-switch").is_none());
}

#[gpui::test]
fn search_shift_cmd_a_from_toolbar_field_flips_rules(cx: &mut TestAppContext) {
    let root = fixture("search-switch-key");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    focus_search_input(&view, &mut cx);
    cx.simulate_keystrokes("cmd-shift-a");
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    assert!(search.read_with(&cx, |search, _| search.everything()));
}

#[gpui::test]
fn search_opened_with_everything_shows_switch_off(cx: &mut TestAppContext) {
    let root = fixture("search-open-everything");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-shift-a");
    settle(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    assert!(search.read_with(&cx, |search, _| search.everything()));
    assert!(cx.debug_bounds("search-skip-switch").is_some());
}

#[gpui::test]
fn search_shift_cmd_m_cycles_the_match_mode(cx: &mut TestAppContext) {
    let root = fixture("search-cycle");
    std::fs::write(root.join("needle.txt"), "needle\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    focus_search_input(&view, &mut cx);
    cx.simulate_input("needle");
    wait_for_search(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    focus_search_input(&view, &mut cx);

    cx.simulate_keystrokes("cmd-shift-m");
    assert_eq!(search.read_with(&cx, |search, _| search.matching()), crate::search::Match::Names);
    focus_search_input(&view, &mut cx);
    cx.simulate_keystrokes("cmd-shift-m");
    assert_eq!(search.read_with(&cx, |search, _| search.matching()), crate::search::Match::Contents);
    focus_search_input(&view, &mut cx);
    cx.simulate_keystrokes("cmd-shift-m");
    assert_eq!(search.read_with(&cx, |search, _| search.matching()), crate::search::Match::All);
}

#[gpui::test]
fn search_idle_example_fills_the_query(cx: &mut TestAppContext) {
    let root = fixture("search-idle");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    settle(&view, &mut cx);
    click_selector(&mut cx, "search-example-0");
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    assert_eq!(search.read_with(&cx, |search, cx| search.query_text(cx)), "config");
    let focus = search.read_with(&cx, |search, cx| search.input_focus_handle(cx));
    assert!(cx.update(|window, _| focus.is_focused(window)), "focus returns to the field");
}

#[gpui::test]
fn search_idle_example_this_mac_switches_scope(cx: &mut TestAppContext) {
    let root = fixture("search-idle-scope");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    settle(&view, &mut cx);
    click_selector(&mut cx, "search-example-2");
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    assert_eq!(search.read_with(&cx, |search, _| search.scope()), crate::search::Scope::ThisMac);
    assert_eq!(search.read_with(&cx, |search, cx| search.query_text(cx)), "", "the field stays empty");
}

#[gpui::test]
fn search_idle_example_everything_flips_rules(cx: &mut TestAppContext) {
    let root = fixture("search-idle-everything");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    settle(&view, &mut cx);
    click_selector(&mut cx, "search-example-3");
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    assert!(search.read_with(&cx, |search, _| search.everything()));
}

#[gpui::test]
fn search_idle_shows_for_empty_query(cx: &mut TestAppContext) {
    let root = fixture("search-idle-again");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    settle(&view, &mut cx);
    cx.simulate_input("x");
    wait_for_search(&view, &mut cx);
    assert!(cx.debug_bounds("search-example-0").is_none());
    cx.simulate_keystrokes("backspace");
    settle(&view, &mut cx);
    assert!(cx.debug_bounds("search-example-0").is_some());
}

#[gpui::test]
fn search_field_click_opens_search_with_focus_in_the_field(cx: &mut TestAppContext) {
    let root = fixture("toolbar-search-click");
    std::fs::write(root.join("notes.txt"), "needle\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    click_selector(&mut cx, "toolbar-search");
    settle(&view, &mut cx);
    assert!(view.read_with(&cx, |w, _| w.search.is_some()), "the field opens search");
    focus_search_input(&view, &mut cx);
    cx.simulate_input("needle");
    wait_for_search(&view, &mut cx);
    let paths = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search").read(cx).result_paths());
    assert!(paths.contains(&root.join("notes.txt")), "{paths:?}");
}

#[gpui::test]
fn cmd_f_focuses_the_field_when_search_is_open(cx: &mut TestAppContext) {
    let root = fixture("toolbar-cmd-f");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("notes");
    wait_for_search(&view, &mut cx);
    cx.simulate_keystrokes("tab");
    let focus = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search").read(cx).input_focus_handle(cx));
    assert!(!cx.update(|window, _| focus.is_focused(window)), "tab moved focus to the results");
    cx.simulate_keystrokes("cmd-f");
    focus_search_input(&view, &mut cx);
    cx.simulate_input("x");
    let query = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search").read(cx).query_text(cx));
    assert_eq!(query, "x", "the whole query was selected");
}

#[gpui::test]
fn escape_in_the_field_closes_search_and_restores_pane_focus(cx: &mut TestAppContext) {
    let root = fixture("search-close");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("notes");
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(&cx, |w, _| w.search.is_none()));
    cx.simulate_keystrokes("n");
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "type-to-select proves the pane has focus");
}

#[gpui::test]
fn done_button_closes_search(cx: &mut TestAppContext) {
    let root = fixture("search-done");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("notes");
    wait_for_search(&view, &mut cx);
    click_selector(&mut cx, "search-done");
    settle(&view, &mut cx);
    assert!(view.read_with(&cx, |w, _| w.search.is_none()));
    cx.simulate_keystrokes("n");
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"));
}

#[gpui::test]
fn vim_letters_type_into_the_band_field(cx: &mut TestAppContext) {
    let root = fixture("search-vim");
    let (view, mut cx) = open(cx, &root);
    let before = (dir(&view, &cx), selected(&view, &cx));
    use_vim(&view, &mut cx);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_keystrokes("j k / -");
    let query = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search").read(cx).query_text(cx));
    assert_eq!(query, "jk/-");
    assert_eq!((dir(&view, &cx), selected(&view, &cx)), before, "no Vim motion reached the pane");
}

#[gpui::test]
fn go_to_button_opens_the_palette(cx: &mut TestAppContext) {
    let root = fixture("toolbar-go-to");
    let (view, mut cx) = open(cx, &root);
    click_selector(&mut cx, "toolbar-go-to");
    settle(&view, &mut cx);
    let focus = view.read_with(&cx, |w, cx| w.palette.as_ref().expect("palette opens").input.read(cx).focus_handle(cx));
    assert!(cx.update(|window, _| focus.is_focused(window)), "focus is in the palette field");
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(&cx, |w, _| w.palette.is_none()));
    cx.simulate_keystrokes("n");
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "the pane has focus again");
}

#[gpui::test]
fn go_to_button_is_hidden_while_searching(cx: &mut TestAppContext) {
    let root = fixture("toolbar-search-band");
    let (_view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    assert!(cx.debug_bounds("toolbar-go-to").is_none());
    assert!(cx.debug_bounds("toolbar-search").is_none());
    assert!(cx.debug_bounds("search-field").is_some(), "the band's own field is there instead");
}

#[gpui::test]
fn match_names_and_contents_change_the_results(cx: &mut TestAppContext) {
    let root = fixture("search-match");
    std::fs::write(root.join("alpha/needle.txt"), "nothing\n").expect("create fixture");
    std::fs::write(root.join("beta/other.txt"), "needle in beta\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("needle");
    wait_for_search(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    let has = |cx: &VisualTestContext, path: &Path| {
        search.read_with(cx, |search, _| search.result_paths().iter().any(|p| p == path))
    };
    let (name_hit, content_hit) = (root.join("alpha/needle.txt"), root.join("beta/other.txt"));
    assert!(has(&cx, &name_hit) && has(&cx, &content_hit), "All searches names and contents");

    click_selector(&mut cx, "search-match-names");
    wait_for_search(&view, &mut cx);
    assert!(has(&cx, &name_hit), "the name hit stays");
    assert!(!has(&cx, &content_hit), "no file is read, so the content hit is gone");
    focus_search_input(&view, &mut cx);

    click_selector(&mut cx, "search-match-contents");
    wait_for_search(&view, &mut cx);
    assert!(!has(&cx, &name_hit), "name-only hits are dropped");
    assert!(has(&cx, &content_hit));
    focus_search_input(&view, &mut cx);

    click_selector(&mut cx, "search-match-all");
    wait_for_search(&view, &mut cx);
    assert!(has(&cx, &name_hit) && has(&cx, &content_hit));
    focus_search_input(&view, &mut cx);
}

#[gpui::test]
fn spotlight_scope_disables_the_match_control(cx: &mut TestAppContext) {
    use crate::effects::Call;
    let root = fixture("search-match-spotlight");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("notes");
    wait_for_search(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    click_selector(&mut cx, "search-match-contents");
    wait_for_search(&view, &mut cx);
    assert_eq!(search.read_with(&cx, |search, _| search.effective_match()), crate::search::Match::Contents);

    effect_calls(&mut cx);
    click_selector(&mut cx, "search-scope-this-mac");
    wait_for_search(&view, &mut cx);
    let calls = effect_calls(&mut cx);
    assert!(calls.iter().any(|call| matches!(call, Call::Spotlight(query) if query == "notes")), "{calls:?}");
    assert_eq!(
        search.read_with(&cx, |search, _| search.effective_match()),
        crate::search::Match::Names,
        "Spotlight searches names only"
    );
    click_selector(&mut cx, "search-match-all");
    assert_eq!(
        search.read_with(&cx, |search, _| search.matching()),
        crate::search::Match::Contents,
        "the stored choice survives, and clicks on Spotlight change nothing"
    );
    click_selector(&mut cx, "search-scope-folder");
    settle(&view, &mut cx);
    assert_eq!(search.read_with(&cx, |search, _| search.effective_match()), crate::search::Match::Contents);
}

#[gpui::test]
fn switching_tabs_closes_search(cx: &mut TestAppContext) {
    let root = fixture("search-tabs");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-t");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("notes");
    wait_for_search(&view, &mut cx);
    cx.simulate_keystrokes("cmd-shift-]");
    settle(&view, &mut cx);
    assert!(view.read_with(&cx, |w, _| w.search.is_none()), "switching tabs closes search");
    cx.simulate_keystrokes("n");
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "the new tab's pane has focus");
}

#[gpui::test]
fn clicking_the_active_tab_keeps_search_open(cx: &mut TestAppContext) {
    let root = fixture("search-tab-click");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-t");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("notes");
    let bounds = cx.debug_bounds("tab-1").expect("active tab on screen");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    settle(&view, &mut cx);
    assert!(view.read_with(&cx, |w, _| w.search.is_some()), "the active tab is not a switch");
    let bounds = cx.debug_bounds("tab-0").expect("other tab on screen");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    settle(&view, &mut cx);
    assert!(view.read_with(&cx, |w, _| w.search.is_none()), "switching tabs closes search");
}

fn sidebar_focused(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) -> bool {
    let focus = view.read_with(cx, |w, cx| w.sidebar.read(cx).focus_handle(cx));
    cx.update(|window, _| focus.is_focused(window))
}

fn search_input_focused(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) -> bool {
    let focus = view.read_with(cx, |w, cx| w.search.as_ref().expect("search open").read(cx).input_focus_handle(cx));
    cx.update(|window, _| focus.is_focused(window))
}

fn search_results_focused(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) -> bool {
    let focus = view.read_with(cx, |w, cx| w.search.as_ref().expect("search open").read(cx).results_focus_handle());
    cx.update(|window, _| focus.is_focused(window))
}

#[gpui::test]
fn search_down_from_field_focuses_results(cx: &mut TestAppContext) {
    let root = fixture("search-down-field");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("a");
    wait_for_search(&view, &mut cx);
    let paths = view.read_with(&cx, |w, cx| {
        let search = w.search.as_ref().expect("search open").read(cx);
        assert!(search.result_count() >= 2, "two name hits: {}", search.result_count());
        search.result_paths()
    });

    cx.simulate_keystrokes("down");
    assert!(search_results_focused(&view, &mut cx), "↓ moves into the results");
    let (cursor, selected) = view.read_with(&cx, |w, cx| {
        let search = w.search.as_ref().expect("search open").read(cx);
        (search.cursor(), search.selected_entry().map(|(path, _)| path))
    });
    assert_eq!(cursor, 0, "row 0 unless the user moved");
    assert_eq!(selected.as_deref(), Some(paths[0].as_path()), "row 0 is selected");

    cx.simulate_keystrokes("down");
    let (cursor, selected) = view.read_with(&cx, |w, cx| {
        let search = w.search.as_ref().expect("search open").read(cx);
        (search.cursor(), search.selected_entry().map(|(path, _)| path))
    });
    assert_eq!(cursor, 1, "↓ again moves on");
    assert_eq!(selected.as_deref(), Some(paths[1].as_path()));
}

#[gpui::test]
fn search_return_in_the_field_moves_to_the_first_result(cx: &mut TestAppContext) {
    let root = fixture("search-return-field");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("alpha");
    wait_for_search(&view, &mut cx);
    let first = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search open").read(cx).result_paths()[0].clone());

    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root, "Return in the field opens nothing");
    assert!(search_results_focused(&view, &mut cx), "Return moves into the results");
    let selected = view.read_with(&cx, |w, cx| {
        w.search.as_ref().expect("search open").read(cx).selected_entry().map(|(path, _)| path)
    });
    assert_eq!(selected.as_deref(), Some(first.as_path()), "on the first result");

    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"), "Return in the results opens it");
}

#[gpui::test]
fn cmd_backspace_clears_the_search_field(cx: &mut TestAppContext) {
    let root = fixture("search-cmd-backspace");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("needle");
    wait_for_search(&view, &mut cx);
    let query = |cx: &VisualTestContext| {
        view.read_with(cx, |w, cx| w.search.as_ref().expect("search open").read(cx).query_text(cx))
    };
    assert_eq!(query(&cx), "needle");
    cx.simulate_keystrokes("cmd-backspace");
    assert_eq!(query(&cx), "", "Cmd-Backspace clears the field");
    assert!(root.join("notes.txt").exists(), "Cmd-Backspace never trashes a file from the search field");
}

#[gpui::test]
fn search_down_without_results_stays_in_the_field(cx: &mut TestAppContext) {
    let root = fixture("search-down-empty");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("zzzzz");
    wait_for_search(&view, &mut cx);
    assert_eq!(view.read_with(&cx, |w, cx| w.search.as_ref().expect("search open").read(cx).result_count()), 0);
    cx.simulate_keystrokes("down");
    assert!(search_input_focused(&view, &mut cx), "↓ does nothing without results");
}

#[gpui::test]
fn search_up_from_first_row_returns_to_field(cx: &mut TestAppContext) {
    let root = fixture("search-up-field");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("a");
    wait_for_search(&view, &mut cx);
    cx.simulate_keystrokes("down");
    assert!(search_results_focused(&view, &mut cx));
    cx.simulate_keystrokes("up");
    assert!(search_input_focused(&view, &mut cx), "↑ on the first row returns to the field");
    assert_eq!(view.read_with(&cx, |w, cx| w.search.as_ref().expect("search open").read(cx).cursor()), 0);

    cx.simulate_keystrokes("down down");
    assert_eq!(view.read_with(&cx, |w, cx| w.search.as_ref().expect("search open").read(cx).cursor()), 1);
    cx.simulate_keystrokes("up");
    assert!(search_results_focused(&view, &mut cx), "↑ below the first row just moves up");
    assert_eq!(view.read_with(&cx, |w, cx| w.search.as_ref().expect("search open").read(cx).cursor()), 0);
}

#[gpui::test]
fn tab_on_an_empty_search_goes_to_the_sidebar_and_esc_still_closes(cx: &mut TestAppContext) {
    let root = fixture("search-empty-tab");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_keystrokes("tab");
    assert!(sidebar_focused(&view, &mut cx), "no results to visit, so Tab goes to the sidebar");
    cx.simulate_keystrokes("tab");
    assert!(search_input_focused(&view, &mut cx), "Tab from the sidebar comes back to the field");
    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(&cx, |w, _| w.search.is_none()), "Esc in the sidebar closes search");
    cx.simulate_keystrokes("n");
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "the pane has focus");
}

#[gpui::test]
fn tab_cycles_field_results_sidebar_while_searching(cx: &mut TestAppContext) {
    let root = fixture("search-tab-cycle");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("notes");
    wait_for_search(&view, &mut cx);
    cx.simulate_keystrokes("tab");
    assert!(!search_input_focused(&view, &mut cx) && !sidebar_focused(&view, &mut cx), "results");
    cx.simulate_keystrokes("tab");
    assert!(sidebar_focused(&view, &mut cx), "results, then the sidebar");
    cx.simulate_keystrokes("tab");
    assert!(search_input_focused(&view, &mut cx), "the hidden pane is skipped: back to the field");
    cx.simulate_keystrokes("tab tab");
    assert!(sidebar_focused(&view, &mut cx));
    cx.simulate_keystrokes("cmd-f");
    assert!(search_input_focused(&view, &mut cx), "Cmd-F from the sidebar refocuses the field");
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(&cx, |w, _| w.search.is_none()));
}

#[gpui::test]
fn closing_an_overlay_over_search_returns_to_the_field(cx: &mut TestAppContext) {
    let root = fixture("search-overlays");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("notes");
    wait_for_search(&view, &mut cx);
    cx.simulate_keystrokes("cmd-j");
    assert!(view.read_with(&cx, |w, _| w.why.is_some()), "Why This View opens over search");
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(&cx, |w, _| w.why.is_none() && w.search.is_some()));
    assert!(search_input_focused(&view, &mut cx), "focus goes back to the field, not the hidden pane");
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(&cx, |w, _| w.search.is_none()), "Esc in the field still closes search");
}

#[gpui::test]
fn results_emptied_under_focus_fall_back_to_the_field(cx: &mut TestAppContext) {
    let root = fixture("search-results-vanish");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("notes");
    wait_for_search(&view, &mut cx);
    cx.simulate_keystrokes("tab");
    // Rerunning clears the list, so the focused results element leaves the tree.
    click_selector(&mut cx, "search-match-contents");
    settle(&view, &mut cx);
    wait_for_search(&view, &mut cx);
    assert!(search_input_focused(&view, &mut cx), "focus never dies with the list");
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(&cx, |w, _| w.search.is_none()));
}

fn focus_search_input(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) {
    let focus = view.read_with(cx, |w, cx| w.search.as_ref().expect("search open").read(cx).input_focus_handle(cx));
    assert!(cx.update(|window, _| focus.is_focused(window)), "focus is in the search field");
}

#[gpui::test]
fn find_everything_from_the_list_opens_search_with_rules_off(cx: &mut TestAppContext) {
    let root = fixture("find-everything");
    std::fs::create_dir_all(root.join("node_modules/pkg")).expect("create fixture");
    std::fs::write(root.join("node_modules/pkg/needle.txt"), "needle\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);

    cx.simulate_keystrokes("cmd-shift-a");
    cx.run_until_parked();
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    assert!(search.read_with(&cx, |search, _| search.everything()), "every rule is off");
    assert_eq!(search.read_with(&cx, |search, _| search.scope()), crate::search::Scope::Folder);
    focus_search_input(&view, &mut cx);

    cx.simulate_input("needle");
    wait_for_search(&view, &mut cx);
    let paths = search.read_with(&cx, |search, _| search.result_paths());
    assert!(paths.contains(&root.join("node_modules/pkg/needle.txt")), "{paths:?}");
    assert_eq!(search.read_with(&cx, |search, _| search.skipped_count()), 0);
}

#[gpui::test]
fn find_everything_does_nothing_over_the_palette(cx: &mut TestAppContext) {
    let root = fixture("find-everything-palette");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-shift-a");
    cx.run_until_parked();
    assert!(view.read_with(&cx, |w, _| w.search.is_none()), "search stays closed");
    assert!(view.read_with(&cx, |w, _| w.palette.is_some()), "the palette stays open");
}

#[gpui::test]
fn find_everything_from_the_sidebar_with_search_open(cx: &mut TestAppContext) {
    let root = fixture("find-everything-sidebar");
    std::fs::write(root.join("notes.txt"), "needle\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("needle");
    wait_for_search(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    assert!(!search.read_with(&cx, |search, _| search.everything()));

    let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());
    let focus = sidebar.read_with(&cx, |sidebar, cx| sidebar.focus_handle(cx));
    cx.update(|window, cx| window.focus(&focus, cx));
    cx.simulate_keystrokes("cmd-shift-a");
    wait_for_search(&view, &mut cx);
    assert!(search.read_with(&cx, |search, _| search.everything()), "the open search flips its rules");
    focus_search_input(&view, &mut cx);
}

#[gpui::test]
fn open_search_with_a_query_runs_it(cx: &mut TestAppContext) {
    let root = fixture("open-search-query");
    std::fs::write(root.join("notes.txt"), "needle\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    let window = cx.update(|_, cx| cx.windows()[0]);
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.open_search(
                SearchStart { folder: root.clone(), query: "needle".to_owned(), everything: false },
                window,
                cx,
            );
        });
    })
    .expect("open search with a query");
    wait_for_search(&view, &mut cx);

    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    assert_eq!(search.read_with(&cx, |search, cx| search.query_text(cx)), "needle");
    assert!(!search.read_with(&cx, |search, _| search.everything()));
    let paths = search.read_with(&cx, |search, _| search.result_paths());
    assert!(paths.contains(&root.join("notes.txt")), "{paths:?}");
}

fn menu_action(menu: &str, title: &str, cx: &VisualTestContext) -> Box<dyn gpui::Action> {
    cx.read(|cx| {
        let menus = cx.get_menus().expect("menus are installed");
        menus
            .iter()
            .find(|candidate| candidate.name.as_ref() == menu)
            .and_then(|menu| {
                menu.items.iter().find_map(|item| match item {
                    gpui::OwnedMenuItem::Action { name, action, .. } if name == title => Some(action.boxed_clone()),
                    _ => None,
                })
            })
            .unwrap_or_else(|| panic!("{menu} > {title} exists"))
    })
}

fn dispatch_menu(menu: &str, title: &str, cx: &mut VisualTestContext) {
    let action = menu_action(menu, title, cx);
    let window = cx.update(|_, cx| cx.windows()[0]);
    cx.update_window(window, |_, window, cx| window.dispatch_action(action, cx)).expect("dispatch menu item");
    cx.run_until_parked();
}

#[gpui::test]
fn menu_keys_are_the_finder_keys(cx: &mut TestAppContext) {
    let root = fixture("menu-keys");
    let (_view, cx) = open(cx, &root);
    let checks = [
        (crate::palette::CommandId::ShowSearch, "cmd-f"),
        (crate::palette::CommandId::SearchEverything, "cmd-shift-a"),
        (crate::palette::CommandId::OpenParent, "cmd-up"),
        (crate::palette::CommandId::Undo, "cmd-z"),
        (crate::palette::CommandId::Copy, "cmd-c"),
        (crate::palette::CommandId::ShowCommandPalette, "cmd-k"),
        (crate::palette::CommandId::GoToFolder, "cmd-shift-g"),
        (crate::palette::CommandId::ToggleActivity, "alt-cmd-l"),
    ];
    for (id, expected) in checks {
        let actual = cx.read(|cx| {
            let command = crate::palette::COMMANDS.iter().find(|command| command.id == id).expect("command");
            let keymap = cx.key_bindings();
            let keymap = keymap.borrow();
            keymap.bindings_for_action(command.action).next().map(|binding| {
                binding
                    .keystrokes()
                    .iter()
                    .map(|keystroke| keystroke.unparse(gpui::OperatingSystem::Mac))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
        });
        assert_eq!(actual.as_deref(), Some(expected), "{id:?} shows its Finder key");
    }
}

#[gpui::test]
fn menu_items_are_enabled_by_focus(cx: &mut TestAppContext) {
    let root = fixture("menu-focus");
    std::fs::write(root.join("notes.txt"), "needle\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);

    assert!(action_available(&crate::actions::StartFilter, &mut cx), "list: Filter");
    assert!(action_available(&crate::actions::Rename, &mut cx), "list: Rename");
    assert!(action_available(&crate::actions::MoveToTrash, &mut cx), "list: Move to Trash");
    assert!(action_available(&crate::actions::GoBack, &mut cx), "any: Back");

    cx.simulate_keystrokes("tab");
    assert!(!action_available(&crate::actions::StartFilter, &mut cx), "sidebar: no Filter");
    assert!(!action_available(&crate::actions::Rename, &mut cx), "sidebar: no Rename");
    assert!(!action_available(&crate::actions::CopyFiles, &mut cx), "sidebar: no Copy");
    assert!(!action_available(&crate::actions::PasteFiles, &mut cx), "sidebar: no Paste");
    assert!(!action_available(&crate::actions::NewFolder, &mut cx), "sidebar: no New Folder");
    assert!(!action_available(&crate::actions::QuickLook, &mut cx), "sidebar: no Quick Look");
    assert!(action_available(&crate::actions::GoBack, &mut cx), "sidebar: Back stays");
    assert!(action_available(&crate::actions::ShowSearch, &mut cx), "sidebar: Find stays");
    assert!(action_available(&crate::actions::SearchEverything, &mut cx), "sidebar: Find Everything stays");
    assert!(action_available(&crate::actions::Undo, &mut cx), "sidebar: Undo stays");

    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("needle");
    wait_for_search(&view, &mut cx);
    cx.simulate_keystrokes("tab");
    assert!(action_available(&crate::actions::ShowSearch, &mut cx), "results: Find stays available");
    assert!(action_available(&crate::actions::SearchEverything, &mut cx), "results: Find Everything stays");
    assert!(action_available(&crate::actions::CopyFiles, &mut cx), "results: Copy");
    assert!(action_available(&crate::actions::SelectAll, &mut cx), "results: Select All");
    assert!(action_available(&crate::actions::MoveToTrash, &mut cx), "results: Move to Trash");
    assert!(action_available(&crate::actions::QuickLook, &mut cx), "results: Quick Look");
    assert!(!action_available(&crate::actions::PasteFiles, &mut cx), "results: no Paste");
    assert!(!action_available(&crate::actions::StartFilter, &mut cx), "results: no Filter");
}

#[gpui::test]
fn menu_items_dispatch(cx: &mut TestAppContext) {
    let root = fixture("menu-dispatch");
    let (view, mut cx) = open(cx, &root);
    cx.update(|_, cx| crate::menus::init_for_tests(cx));

    cx.simulate_keystrokes("cmd-down");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"));
    dispatch_menu("Go", "Enclosing Folder", &mut cx);
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root);
    dispatch_menu("Go", "Back", &mut cx);
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"));

    dispatch_menu("Edit", "Filter This Folder", &mut cx);
    assert!(pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()), "Filter opens");
    cx.simulate_keystrokes("escape");

    dispatch_menu("Edit", "Find…", &mut cx);
    assert!(view.read_with(&cx, |w, _| w.search.is_some()), "Find opens search");
    cx.simulate_keystrokes("escape");
    dispatch_menu("Go", "Go to…", &mut cx);
    assert!(view.read_with(&cx, |w, _| w.palette.is_some()), "Go to opens the palette");
    cx.simulate_keystrokes("escape");

    let activity = view.read_with(&cx, |w, _| w.show_activity);
    dispatch_menu("View", "Show Activity", &mut cx);
    assert_ne!(view.read_with(&cx, |w, _| w.show_activity), activity, "Show Activity flips");

    dispatch_menu("File", "New Tab", &mut cx);
    assert_eq!(view.read_with(&cx, |w, _| w.tabs.len()), 2, "New Tab opens a tab");
}

#[gpui::test]
fn window_menu_tab_items_switch_tabs(cx: &mut TestAppContext) {
    let root = fixture("menu-tabs");
    let (view, mut cx) = open(cx, &root);
    cx.update(|_, cx| crate::menus::init_for_tests(cx));
    cx.simulate_keystrokes("cmd-t");
    settle(&view, &mut cx);
    assert_eq!(view.read_with(&cx, |w, _| w.tabs.len()), 2);

    dispatch_menu("Window", "Show Previous Tab", &mut cx);
    assert_eq!(view.read_with(&cx, |w, _| w.tabs.active_index()), 0);
    dispatch_menu("Window", "Show Next Tab", &mut cx);
    assert_eq!(view.read_with(&cx, |w, _| w.tabs.active_index()), 1);
}

#[gpui::test]
fn help_opens_palette_in_command_mode(cx: &mut TestAppContext) {
    let root = fixture("menu-help");
    let (view, mut cx) = open(cx, &root);
    let window = cx.update(|_, cx| cx.windows()[0]);
    cx.update_window(window, |_, window, cx| window.dispatch_action(Box::new(crate::actions::KeyboardShortcuts), cx))
        .expect("dispatch Help item");
    cx.run_until_parked();
    let (mode, text, rows) = view.read_with(&cx, |w, cx| {
        let palette = w.palette.as_ref().expect("palette opens");
        (palette.mode, palette.input.read(cx).text().to_owned(), w.palette_rows())
    });
    assert_eq!(mode, PaletteMode::Commands, "the palette starts in command mode");
    assert_eq!(text, "", "the > lives in the token, not the field");
    assert!(rows.iter().any(|row| row == "New Window"), "command rows: {rows:?}");
}

#[gpui::test]
fn search_results_copy_and_trash_the_selection(cx: &mut TestAppContext) {
    let root = fixture("search-actions");
    std::fs::write(root.join("notes.txt"), "needle\n").expect("create fixture");
    std::fs::write(root.join("alpha/needle.txt"), "needle\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("needle");
    wait_for_search(&view, &mut cx);
    cx.simulate_keystrokes("tab cmd-a cmd-c");

    let copied = cx.read(|cx| cx.global::<Operations>().clipboard.as_ref().map(|clip| clip.paths.clone()));
    let copied = copied.expect("copy filled the clipboard");
    assert!(copied.contains(&root.join("notes.txt")), "{copied:?}");
    assert!(copied.contains(&root.join("alpha/needle.txt")), "{copied:?}");

    cx.simulate_keystrokes("cmd-backspace");
    settle(&view, &mut cx);
    assert!(!root.join("notes.txt").exists(), "trash acted on the results");
    assert!(!root.join("alpha/needle.txt").exists());
    assert!(view.read_with(&cx, |w, _| w.search.is_none()), "an operation closes the search");
}

#[gpui::test]
fn search_result_return_navigates_and_closes(cx: &mut TestAppContext) {
    let root = fixture("search-open");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("alpha");
    wait_for_search(&view, &mut cx);

    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"));
    assert!(view.read_with(&cx, |w, _| w.search.is_none()));
}

#[gpui::test]
fn search_result_cmd_r_reveals_the_file(cx: &mut TestAppContext) {
    let root = fixture("search-reveal");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("notes");
    wait_for_search(&view, &mut cx);

    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("cmd-r");
    settle(&view, &mut cx);
    assert!(view.read_with(&cx, |w, _| w.search.is_none()));
    assert_eq!(dir(&view, &cx), root);
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"));
}

#[gpui::test]
fn command_palette_runs_a_command(cx: &mut TestAppContext) {
    let root = fixture("palette-command");
    std::fs::write(root.join(".secret"), "").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    let count = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, cx| p.item_count(cx));
    assert_eq!(count(&cx), 4, "hidden files show by default");

    cx.simulate_keystrokes("cmd-k");
    settle(&view, &mut cx);
    assert!(view.read_with(&cx, |w, _| w.palette.is_some()));
    cx.simulate_input(">toggle hidden");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);

    assert!(view.read_with(&cx, |w, _| w.palette.is_none()), "Return closes the palette");
    assert_eq!(count(&cx), 3, "the command ran");
}

#[gpui::test]
fn command_palette_goes_to_a_recent_folder(cx: &mut TestAppContext) {
    let root = fixture("palette-go");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-down");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"));
    cx.simulate_keystrokes("cmd-up");
    settle(&view, &mut cx);
    assert!(
        cx.read(|cx| cx.global::<Ui>().palette_recents.contains(&root.join("alpha"))),
        "the visit is in the palette's recent list"
    );

    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("alpha");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"));
    assert!(view.read_with(&cx, |w, _| w.palette.is_none()));
}

#[gpui::test]
fn command_palette_cmd_return_opens_a_new_tab(cx: &mut TestAppContext) {
    let root = fixture("palette-tab");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("beta");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-enter");
    settle(&view, &mut cx);

    assert_eq!(view.read_with(&cx, |w, _| w.tabs.len()), 2);
    assert_eq!(dir(&view, &cx), root.join("beta"));
    assert!(view.read_with(&cx, |w, _| w.palette.is_none()));
}

#[gpui::test]
fn command_palette_keys_follow_the_layer(cx: &mut TestAppContext) {
    let root = fixture("palette-keys");
    let (view, mut cx) = open(cx, &root);
    let copy_keys = |cx: &VisualTestContext| {
        view.read_with(cx, |w, cx| {
                let palette = w.palette.as_ref().expect("palette is open");
                let i = palette
                    .listing
                    .items
                    .iter()
                    .position(|item| matches!(item, Item::Command { command, .. } if command.id == crate::palette::CommandId::Copy))
                    .expect("the Copy row is listed");
                palette.row_keys(i, cx).map(|keys| keys.to_string())
            })
    };
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">copy");
    settle(&view, &mut cx);
    assert_eq!(copy_keys(&cx).as_deref(), Some("⌘C"));
    cx.simulate_keystrokes("escape");

    use_vim(&view, &mut cx);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">copy");
    settle(&view, &mut cx);
    assert_eq!(copy_keys(&cx).as_deref(), Some("yy"), "the palette follows keymap = vim");
}

#[gpui::test]
fn vim_letters_type_in_every_text_field(cx: &mut TestAppContext) {
    let root = fixture("vim-typing");
    let (view, mut cx) = open(cx, &root);
    use_vim(&view, &mut cx);

    // The palette's input always has focus; j/k must type, not move its cursor.
    cx.simulate_keystrokes("cmd-k");
    settle(&view, &mut cx);
    cx.simulate_input("jk");
    let palette =
        view.read_with(&cx, |w, cx| w.palette.as_ref().map(|p| (p.input.read(cx).text().to_owned(), p.cursor)));
    assert_eq!(palette, Some(("jk".to_owned(), 0)), "j and k type in the palette");
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);

    // The filter field already excludes the Vim layer via `!Filter`.
    cx.simulate_keystrokes("/");
    cx.simulate_input("jk");
    let filter = pane(&view, &cx).read_with(&cx, |p, cx| p.filter_text(cx));
    assert_eq!(filter.as_deref(), Some("jk"), "j and k type in the filter");
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);

    // A rename field sits inside the list's context, so it needs `!TextInput`.
    cx.simulate_keystrokes("g g");
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.simulate_input("jk");
    let name = pane(&view, &cx).read_with(&cx, |p, cx| p.rename_text(cx));
    assert_eq!(name.as_deref(), Some("jk"), "j and k type in a rename");
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);

    // The search field is its own context and never had a Vim binding.
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("jk");
    let query = view.read_with(&cx, |w, cx| w.search.as_ref().map(|s| s.read(cx).query_text(cx)));
    assert_eq!(query.as_deref(), Some("jk"), "j and k type in the search field");
}

#[gpui::test]
fn palette_lists_sections_in_order(cx: &mut TestAppContext) {
    let root = fixture("palette-sections");
    // gamma stays in "In": alpha becomes Recent and beta is a favorite.
    std::fs::create_dir_all(root.join("gamma")).expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("beta")]));
    cx.simulate_keystrokes("cmd-down");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-up");
    settle(&view, &mut cx);

    cx.simulate_keystrokes("cmd-k");
    settle(&view, &mut cx);
    let rows = view.read_with(&cx, |w, _| w.palette_rows());
    assert_eq!(rows.first().map(String::as_str), Some("# Recent"), "{rows:?}");
    let favorites = rows.iter().position(|row| row == "# Favorites").expect("a Favorites header");
    let here = rows.iter().position(|row| row.starts_with("# In ")).expect("an In header");
    assert!(favorites < here, "Recent, then Favorites, then In: {rows:?}");
    assert_eq!(rows[here], format!("# In {}", root.file_name().unwrap().to_string_lossy()));
    assert!(rows[1..favorites].iter().any(|row| row.ends_with("/alpha")), "alpha is Recent: {rows:?}");
    assert_eq!(rows.iter().filter(|row| row.ends_with("/beta")).count(), 1, "beta once only: {rows:?}");
    assert!(rows[favorites + 1..here].iter().any(|row| row.ends_with("/beta")), "under Favorites: {rows:?}");
    assert!(rows[here + 1..].iter().any(|row| row.ends_with("/gamma")), "gamma is In: {rows:?}");
}

#[gpui::test]
fn palette_home_row_names_the_account_and_keeps_the_tilde_path(cx: &mut TestAppContext) {
    let root = fixture("palette-home");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    settle(&view, &mut cx);

    let (rows, names) = view.read_with(&cx, |w, _| (w.palette_rows(), w.palette_row_names()));
    let home = rows.iter().position(|row| row == "~").expect("the home row");
    let home_path = cx.read(|cx| cx.global::<Ui>().home.clone());
    let expected = crate::style::display_name(&home_path);
    let expected = if expected.is_empty() { "Home".to_owned() } else { expected };
    assert_eq!(names[home], expected, "the name line is the account's short name");
    assert_ne!(names[home], "~", "the name is not the path");
    assert_eq!(rows[home], "~", "the path line stays ~");
}

#[gpui::test]
fn palette_twins_get_distinct_labels(cx: &mut TestAppContext) {
    let root = fixture("palette-match");
    for side in ["a", "b"] {
        std::fs::create_dir_all(root.join(side).join("twin")).expect("create fixture");
    }
    let (view, mut cx) = open(cx, &root);
    for side in ["a", "b"] {
        let twin = root.join(side).join("twin");
        pane(&view, &cx).update(&mut cx, |pane, cx| pane.navigate(twin, cx));
        settle(&view, &mut cx);
    }
    // Back to root: the folder you are in is left out of the palette.
    pane(&view, &cx).update(&mut cx, |pane, cx| pane.navigate(root.clone(), cx));
    settle(&view, &mut cx);

    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("twin");
    settle(&view, &mut cx);
    let rows = view.read_with(&cx, |w, _| w.palette_rows());
    assert_eq!(rows.len(), 2, "two folders named twin: {rows:?}");
    assert!(rows[0].ends_with("/a/twin") && rows[1].ends_with("/b/twin"), "the paths tell them apart: {rows:?}");
}

#[gpui::test]
fn palette_leaves_out_the_current_folder(cx: &mut TestAppContext) {
    let root = fixture("palette-here");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-down");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-up");
    settle(&view, &mut cx);
    assert!(cx.read(|cx| cx.global::<Ui>().palette_recents.contains(&root)), "the folder we are in is a recent visit");

    cx.simulate_keystrokes("cmd-k");
    settle(&view, &mut cx);
    let rows = view.read_with(&cx, |w, _| w.palette_rows());
    assert_eq!(rows.first().map(String::as_str), Some("# Recent"), "{rows:?}");
    assert!(rows[1].ends_with("/alpha"), "the first row is the folder before this one: {rows:?}");
    let label = crow_core::abbreviate_home(&root, &cx.read(|cx| cx.global::<Ui>().home.clone()));
    assert!(!rows.contains(&label), "the current folder is not a row: {rows:?}");
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"), "Cmd-K then Return goes somewhere");
}

#[gpui::test]
fn palette_rows_are_inset_from_the_sheet(cx: &mut TestAppContext) {
    let root = fixture("palette-inset");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    settle(&view, &mut cx);
    let sheet = cx.debug_bounds("sheet-panel").expect("the palette sheet");
    let row = cx.debug_bounds("palette-row-0").expect("the first row");
    assert_eq!(row.left() - sheet.left(), px(PALETTE_LIST_PAD + 1.), "list padding plus the sheet's border");
    assert_eq!(sheet.right() - row.right(), px(PALETTE_LIST_PAD + 1.), "the same on the right");
}

#[gpui::test]
fn palette_header_leaves_room_above_the_first_row(cx: &mut TestAppContext) {
    let root = fixture("palette-header-gap");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    settle(&view, &mut cx);
    let label = cx.debug_bounds("palette-header-label-0").expect("the first header");
    let row = cx.debug_bounds("palette-row-0").expect("the first row");
    assert!(label.bottom() <= row.top(), "the header label sits above the row: {label:?} {row:?}");
    assert!(row.top() - label.bottom() >= px(6.), "room between the label and the row: {label:?} {row:?}");
}

#[gpui::test]
fn palette_count_says_places(cx: &mut TestAppContext) {
    let root = fixture("palette-count");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("alpha");
    settle(&view, &mut cx);
    let (rows, count) =
        view.read_with(&cx, |w, _| (w.palette_rows(), w.palette.as_ref().map(|p| p.listing.count.to_string())));
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(count.as_deref(), Some("1 place"));
}

#[gpui::test]
fn palette_cursor_skips_headers(cx: &mut TestAppContext) {
    let root = fixture("palette-cursor");
    std::fs::create_dir_all(root.join("gamma")).expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("beta")]));
    cx.simulate_keystrokes("cmd-down");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-up");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-k");
    settle(&view, &mut cx);

    // Recent holds alpha (root is where we are, so it is left out): item 1 is
    // the first favorite.
    cx.simulate_keystrokes("down");
    let cursor = view.read_with(&cx, |w, _| {
        let palette = w.palette.as_ref()?;
        match palette.listing.items.get(palette.cursor)? {
            Item::Folder { label, .. } => Some(label.clone()),
            Item::Command { .. } => None,
        }
    });
    assert!(cursor.as_deref().is_some_and(|label| label.ends_with("/beta")), "down crossed the header: {cursor:?}");
    cx.simulate_keystrokes("up up up");
    assert_eq!(view.read_with(&cx, |w, _| w.palette.as_ref().map(|p| p.cursor)), Some(0), "up at the top stays");
}

#[gpui::test]
fn palette_gt_becomes_commands_token(cx: &mut TestAppContext) {
    let root = fixture("palette-token");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">");
    settle(&view, &mut cx);
    let (mode, text, count) = view.read_with(&cx, |w, cx| {
        let palette = w.palette.as_ref().expect("palette is open");
        (palette.mode, palette.input.read(cx).text().to_owned(), palette.listing.count.to_string())
    });
    assert_eq!(mode, PaletteMode::Commands, "> switches to commands");
    assert_eq!(text, "", "the > is the token, not text");
    assert!(count.ends_with("commands"), "a command count: {count}");

    cx.simulate_input("hid");
    settle(&view, &mut cx);
    let (first, count) = view.read_with(&cx, |w, _| {
        let palette = w.palette.as_ref().expect("palette is open");
        let first = match &palette.listing.items[0] {
            Item::Command { command, .. } => command.label,
            Item::Folder { .. } => "",
        };
        (first, palette.listing.count.to_string())
    });
    assert!(first.contains("Hidden"), "the hidden-files command comes first: {first}");
    let (matches, total) = count.split_once(" of ").expect("m of n");
    assert!(matches.parse::<usize>().is_ok() && total.parse::<usize>().is_ok(), "m of n: {count}");
}

#[gpui::test]
fn palette_backspace_on_empty_query_removes_token(cx: &mut TestAppContext) {
    let root = fixture("palette-token-back");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("backspace");
    settle(&view, &mut cx);
    let (mode, text, rows) = view.read_with(&cx, |w, cx| {
        let palette = w.palette.as_ref().expect("palette is open");
        (palette.mode, palette.input.read(cx).text().to_owned(), w.palette_rows())
    });
    assert_eq!(mode, PaletteMode::Folders);
    assert_eq!(text, "");
    assert!(rows.first().is_some_and(|row| row.starts_with("# ")), "folders are back: {rows:?}");
}

#[gpui::test]
fn palette_backspace_with_text_keeps_token(cx: &mut TestAppContext) {
    let root = fixture("palette-token-text");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">ab");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("left left backspace");
    settle(&view, &mut cx);
    let (mode, text) = view.read_with(&cx, |w, cx| {
        let palette = w.palette.as_ref().expect("palette is open");
        (palette.mode, palette.input.read(cx).text().to_owned())
    });
    assert_eq!(mode, PaletteMode::Commands, "text keeps the token");
    assert_eq!(text, "ab");
}

#[gpui::test]
fn palette_toggle_chip_shows_state(cx: &mut TestAppContext) {
    let root = fixture("palette-toggle");
    let (view, mut cx) = open(cx, &root);
    let state = |cx: &VisualTestContext| {
        view.read_with(cx, |w, cx| {
            let palette = w.palette.as_ref()?;
            palette.listing.items.iter().find_map(|item| match item {
                Item::Command { command, .. } => w.toggle_state(command.id, cx),
                Item::Folder { .. } => None,
            })
        })
    };
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">hidden");
    settle(&view, &mut cx);
    assert_eq!(state(&cx), Some(true), "hidden files are on by default");

    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">hidden");
    settle(&view, &mut cx);
    assert_eq!(state(&cx), Some(false), "the command turned them off");
}

#[gpui::test]
fn palette_tab_completes_highlighted_folder(cx: &mut TestAppContext) {
    let root = fixture("palette-tab-complete");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-down");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-up");
    settle(&view, &mut cx);

    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("alpha");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("tab");
    settle_palette(&view, &mut cx);
    let text = view.read_with(&cx, |w, cx| w.palette.as_ref().unwrap().input.read(cx).text().to_owned());
    assert_eq!(text, format!("{}/alpha/", root.display()), "the highlighted folder and a slash");
    let rows = view.read_with(&cx, |w, _| w.palette_rows());
    assert!(rows.iter().any(|row| row.ends_with("/alpha/inner")), "its subfolders: {rows:?}");

    cx.simulate_input("inn");
    settle_palette(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha/inner"));
}

#[gpui::test]
fn palette_tab_does_nothing_in_command_mode(cx: &mut TestAppContext) {
    let root = fixture("palette-tab-commands");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("tab");
    settle(&view, &mut cx);
    let (mode, text, open) = view.read_with(&cx, |w, cx| {
        (
            w.palette.as_ref().map(|p| p.mode),
            w.palette.as_ref().map(|p| p.input.read(cx).text().to_owned()),
            w.palette.is_some(),
        )
    });
    assert_eq!(mode, Some(PaletteMode::Commands));
    assert_eq!(text.as_deref(), Some(""), "Tab did not type anything");
    assert!(open, "the palette stays open");
}

#[gpui::test]
fn palette_path_mode_lists_children_off_main_thread(cx: &mut TestAppContext) {
    let root = fixture("palette-path");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(&format!("{}/", root.display()));
    settle_palette(&view, &mut cx);
    let rows = view.read_with(&cx, |w, _| w.palette_rows());
    assert_eq!(rows.len(), 4, "this folder, alpha, beta — no notes.txt: {rows:?}");
    assert!(rows[0].starts_with("# In "), "{rows:?}");
    assert_eq!(rows[1], root.display().to_string(), "the parent itself comes first");
    assert!(rows[2].ends_with("/alpha") && rows[3].ends_with("/beta"), "{rows:?}");
}

#[gpui::test]
fn palette_reveals_a_pasted_file_path_however_it_is_quoted(cx: &mut TestAppContext) {
    let root = fixture("palette-pasted-file");
    std::fs::write(root.join("alpha/inner/My File.txt"), "x").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    let file = root.join("alpha/inner/My File.txt").display().to_string();
    let pasted = [
        file.clone(),
        format!("'{file}'"),
        format!("\"{file}\""),
        format!("  {file} "),
        file.replace(' ', "\\ "),
        format!("file://{}", file.replace(' ', "%20")),
    ];
    for text in pasted {
        pane(&view, &cx).update(&mut cx, |p, cx| p.navigate(root.join("beta"), cx));
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(&text);
        settle_palette(&view, &mut cx);
        let rows = view.read_with(&cx, |w, _| w.palette_rows());
        assert!(rows.get(1).is_some_and(|row| row.ends_with("/My File.txt")), "{text:?} lists the file: {rows:?}");
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        assert!(view.read_with(&cx, |w, _| w.palette.is_none()), "{text:?} closes the palette");
        assert_eq!(dir(&view, &cx), root.join("alpha/inner"), "{text:?} goes to the file's folder");
        assert_eq!(selected_names(&view, &cx), ["My File.txt"], "{text:?} selects the file");
    }
}

#[gpui::test]
fn palette_tab_on_a_file_row_completes_its_name_without_a_slash(cx: &mut TestAppContext) {
    let root = fixture("palette-file-tab");
    let (view, mut cx) = open(cx, &root);
    pane(&view, &cx).update(&mut cx, |p, cx| p.navigate(root.join("beta"), cx));
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(&format!("{}/no", root.display()));
    settle_palette(&view, &mut cx);
    let rows = view.read_with(&cx, |w, _| w.palette_rows());
    assert_eq!(rows.len(), 2, "the header and notes.txt: {rows:?}");
    cx.simulate_keystrokes("tab");
    settle_palette(&view, &mut cx);
    let text = view.read_with(&cx, |w, cx| w.palette.as_ref().map(|p| p.input.read(cx).text().to_owned()));
    assert_eq!(text, Some(format!("{}/notes.txt", root.display())), "no slash after a file");
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root);
    assert_eq!(selected_names(&view, &cx), ["notes.txt"]);
}

#[gpui::test]
fn palette_path_mode_hides_dot_folders_until_typed(cx: &mut TestAppContext) {
    let root = fixture("palette-path-hidden");
    std::fs::create_dir_all(root.join(".hidden")).expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-shift-.");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(&format!("{}/", root.display()));
    settle_palette(&view, &mut cx);
    let rows = view.read_with(&cx, |w, _| w.palette_rows());
    assert!(!rows.iter().any(|row| row.ends_with("/.hidden")), "hidden until asked: {rows:?}");

    cx.simulate_input(".h");
    settle_palette(&view, &mut cx);
    let rows = view.read_with(&cx, |w, _| w.palette_rows());
    assert!(rows.iter().any(|row| row.ends_with("/.hidden")), "a dot partial shows hidden folders: {rows:?}");
}

#[gpui::test]
fn palette_lists_recently_used_commands(cx: &mut TestAppContext) {
    let root = fixture("palette-recent-commands");
    let (view, mut cx) = open(cx, &root);
    for query in ["columns", "why this"] {
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(&format!(">{query}"));
        settle(&view, &mut cx);
        cx.simulate_keystrokes("enter");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
    }

    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">");
    settle(&view, &mut cx);
    let rows = view.read_with(&cx, |w, _| w.palette_rows());
    let recent = rows.iter().position(|row| row == "# Recently used").expect("a Recently used header");
    let all = rows.iter().position(|row| row == "# All commands").expect("an All commands header");
    assert!(recent < all, "{rows:?}");
    assert_eq!(&rows[recent + 1..all], ["Why This View?", "View as Columns"], "newest first: {rows:?}");
    let repeated = rows[all + 1..].iter().filter(|row| *row == "Why This View?" || *row == "View as Columns");
    assert_eq!(repeated.count(), 0, "recent commands are not listed again: {rows:?}");
}

#[gpui::test]
fn palette_recent_commands_survive_relaunch(cx: &mut TestAppContext) {
    let root = fixture("palette-commands-relaunch");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">columns");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);

    let key = crate::palette::CommandId::ViewAsColumns.key();
    let state = cx.update(|_, cx| cx.global::<Ui>().state_dir.clone());
    let file = state.join("palette-commands.json");
    let mut saved = String::new();
    for _ in 0..200 {
        cx.run_until_parked();
        if let Ok(text) = std::fs::read_to_string(&file) {
            saved = text;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(saved.contains(key), "saved with its stable key: {saved:?}");

    // Forget it in memory, then reload the state file as a relaunch would.
    cx.update(|_, cx| cx.global_mut::<Ui>().recent_commands = Default::default());
    cx.update(|_, cx| crate::ui::reload_state_for_tests(cx));
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">");
    settle(&view, &mut cx);
    let rows = view.read_with(&cx, |w, _| w.palette_rows());
    assert_eq!(rows.first().map(String::as_str), Some("# Recently used"), "{rows:?}");
    assert_eq!(rows.get(1).map(String::as_str), Some("View as Columns"), "{rows:?}");
}

#[gpui::test]
fn palette_recent_commands_setting_false_hides_and_stops_recording(cx: &mut TestAppContext) {
    let root = fixture("palette-commands-setting");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">columns");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    let state = cx.update(|_, cx| cx.global::<Ui>().state_dir.clone());
    let file = state.join("palette-commands.json");
    let mut before = String::new();
    for _ in 0..200 {
        cx.run_until_parked();
        if let Ok(text) = std::fs::read_to_string(&file) {
            before = text;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!before.is_empty(), "one command is remembered");

    write_config(&view, &mut cx, "settings.toml", "palette_recent_commands = false\n");
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">why this");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    for _ in 0..20 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">");
    settle(&view, &mut cx);
    let rows = view.read_with(&cx, |w, _| w.palette_rows());
    assert!(!rows.iter().any(|row| row == "# Recently used"), "the section is hidden: {rows:?}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before, "nothing new was recorded");

    cx.simulate_keystrokes("escape");
    write_config(&view, &mut cx, "settings.toml", "");
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">");
    settle(&view, &mut cx);
    let rows = view.read_with(&cx, |w, _| w.palette_rows());
    assert_eq!(rows.first().map(String::as_str), Some("# Recently used"), "{rows:?}");
    assert_eq!(rows.get(1).map(String::as_str), Some("View as Columns"), "only the earlier run: {rows:?}");
}

#[gpui::test]
fn theme_picker_previews_reverts_and_saves(cx: &mut TestAppContext) {
    theme_index();
    let root = fixture("themes");
    let (view, mut cx) = open(cx, &root);
    let theme = |cx: &VisualTestContext| cx.read(|cx| cx.global::<Ui>().theme.name.clone());
    let start = theme(&cx);

    cx.simulate_keystrokes("cmd-alt-t");
    settle(&view, &mut cx);
    assert!(view.read_with(&cx, |w, _| w.picker.is_some()));
    // System, then FireBox (dark), then Paper (light).
    cx.simulate_keystrokes("down down");
    assert_eq!(theme(&cx), "Paper", "moving the cursor shows the theme");
    cx.simulate_keystrokes("escape");
    assert_eq!(theme(&cx), start, "Escape goes back");

    cx.simulate_keystrokes("cmd-alt-t");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("down down enter");
    settle(&view, &mut cx);
    assert_eq!(theme(&cx), "Paper");
    let settings = std::fs::read_to_string(config_dir(&cx).join("settings.toml")).unwrap();
    assert!(settings.contains("theme = \"paper\""), "{settings}");
    assert!(view.read_with(&cx, |w, _| w.picker.is_none()));
}

#[gpui::test]
fn why_sheet_keeps_focus_on_arrows_and_tab(cx: &mut TestAppContext) {
    let root = fixture("why-focus");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-j");
    let focus = view.read_with(&cx, |w, _| w.why.clone()).expect("Why opens");
    cx.simulate_keystrokes("right down tab");
    assert!(view.read_with(&cx, |w, _| w.why.is_some()), "the sheet stays open");
    assert_eq!(cx.update(|window, cx| window.focused(cx)), Some(focus), "focus stays in the sheet");
    let found = violations(&view, &mut cx);
    assert!(found.is_empty(), "{found:?}");
}

#[gpui::test]
fn a_pending_rename_does_not_take_focus_from_an_overlay(cx: &mut TestAppContext) {
    let root = fixture("overlay-focus-rename");
    let (view, mut cx) = open(cx, &root);
    let pane = pane(&view, &cx);
    // A new item's row is not listed yet, so its rename waits for the load —
    // the state ⌘⇧N/⌥⌘N leaves while a big folder reloads.
    let path = root.join("later.txt");
    pane.update(&mut cx, |pane, cx| pane.show_results(vec![path.clone()], true, cx));
    cx.simulate_keystrokes("cmd-j");
    std::fs::write(&path, "x").expect("create the file");
    pane.update(&mut cx, |pane, cx| {
        let folder = pane.test_folder();
        folder.update(cx, |folder, cx| folder.reload(cx));
    });
    settle(&view, &mut cx);
    let why = view.read_with(&cx, |w, _| w.why.clone()).expect("Why opens");
    cx.simulate_keystrokes("down");
    assert_eq!(cx.update(|window, cx| window.focused(cx)), Some(why), "focus stays in the sheet");
    assert!(violations(&view, &mut cx).is_empty());
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);

    // The theme picker, the other overlay a pending rename raced in the fuzz runs.
    cx.simulate_keystrokes("alt-cmd-t");
    settle(&view, &mut cx);
    let picker = view.read_with(&cx, |w, _| w.picker.as_ref().map(|p| p.focus.clone())).expect("the picker opens");
    let path = root.join("later-2.txt");
    pane.update(&mut cx, |pane, cx| pane.show_results(vec![path.clone()], true, cx));
    std::fs::write(&path, "x").expect("create the file");
    pane.update(&mut cx, |pane, cx| {
        let folder = pane.test_folder();
        folder.update(cx, |folder, cx| folder.reload(cx));
    });
    settle(&view, &mut cx);
    cx.simulate_keystrokes("down");
    assert_eq!(cx.update(|window, cx| window.focused(cx)), Some(picker), "focus stays in the picker");
    assert!(violations(&view, &mut cx).is_empty());
}

/// Guards the `NoAction` list in `actions.rs`: every key that acts on the
/// window must do nothing while a sheet is open, so a new window binding
/// without a `Sheet` block fails here.
#[gpui::test]
fn sheets_block_every_window_key(cx: &mut TestAppContext) {
    use crate::keys::{KeyScope, context_stack};
    let root = fixture("sheet-keys");
    let (_view, mut cx) = open(cx, &root);
    // App-level keys still work with a sheet open, as in macOS.
    let allowed = ["cmd-m", "cmd-h", "alt-cmd-h", "cmd-q"];
    let leaks = cx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let mut leaks = Vec::new();
        for layer in [crow_core::KeymapLayer::Finder, crow_core::KeymapLayer::Vim] {
            let window = context_stack(KeyScope::Window, layer);
            let sheet = context_stack(KeyScope::Sheet, layer);
            for binding in keymap.bindings() {
                let keys = binding.keystrokes().iter().map(|key| key.inner().unparse(gpui::OperatingSystem::Mac));
                let keys = keys.collect::<Vec<_>>().join(" ");
                if allowed.contains(&keys.as_str()) {
                    continue;
                }
                let wins = |stack: &[gpui::KeyContext]| {
                    let (found, _) = keymap.bindings_for_input(binding.keystrokes(), stack);
                    found.first().is_some_and(|found| found.action().partial_eq(binding.action()))
                };
                if wins(&window) && wins(&sheet) {
                    leaks.push(format!("{keys} ({})", binding.action().name()));
                }
            }
        }
        leaks.sort();
        leaks.dedup();
        leaks
    });
    assert!(leaks.is_empty(), "window keys act behind a sheet; add a Sheet NoAction in actions.rs: {leaks:?}");
}

#[gpui::test]
fn tab_stays_in_the_theme_picker(cx: &mut TestAppContext) {
    theme_index();
    let root = fixture("picker-tab");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-alt-t");
    settle(&view, &mut cx);
    let focus = view.read_with(&cx, |w, _| w.picker.as_ref().map(|p| p.focus.clone())).expect("picker open");

    cx.simulate_keystrokes("tab");
    assert!(view.read_with(&cx, |w, _| w.picker.is_some()), "Tab keeps the picker open");
    assert_eq!(cx.update(|window, cx| window.focused(cx)), Some(focus.clone()), "Tab stays in the picker");

    cx.simulate_keystrokes("shift-tab");
    assert_eq!(cx.update(|window, cx| window.focused(cx)), Some(focus.clone()), "Shift-Tab stays in the picker");

    // Window-level keys act on the window behind the sheet; they must not here.
    cx.simulate_keystrokes("cmd-shift-[ cmd-t cmd-k");
    assert!(view.read_with(&cx, |w, _| w.picker.is_some()), "the picker stays open");
    assert!(view.read_with(&cx, |w, _| w.palette.is_none()), "Cmd-K does not open the palette behind it");
    assert_eq!(cx.update(|window, cx| window.focused(cx)), Some(focus), "focus stays in the picker");
    let found = violations(&view, &mut cx);
    assert!(found.is_empty(), "{found:?}");
}

#[gpui::test]
fn parent_selects_where_you_came_from_and_history_works(cx: &mut TestAppContext) {
    let root = fixture("history");
    let (view, mut cx) = open(cx, &root.join("alpha"));

    cx.simulate_keystrokes("cmd-up");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root);
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "Going up selects the folder you came from");

    cx.simulate_keystrokes("down cmd-down");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("beta"));

    cx.simulate_keystrokes("cmd-[");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root);
    assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "Back selects the folder you left");

    cx.simulate_keystrokes("cmd-]");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("beta"));
}

#[gpui::test]
fn tabs_open_next_to_the_active_one_and_close(cx: &mut TestAppContext) {
    let root = fixture("tabs");
    let (view, mut cx) = open(cx, &root);

    cx.simulate_keystrokes("cmd-t");
    settle(&view, &mut cx);
    assert_eq!(view.read_with(&cx, |w, _| w.tabs.len()), 2);
    assert_eq!(dir(&view, &cx), root, "A new tab opens on the current folder");

    cx.simulate_keystrokes("down cmd-down");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-shift-[");
    assert_eq!(dir(&view, &cx), root, "Each tab keeps its own folder");

    cx.simulate_keystrokes("cmd-w");
    assert_eq!(view.read_with(&cx, |w, _| w.tabs.len()), 1);
}

#[gpui::test]
fn closing_the_last_window_leaves_the_app_running(app: &mut TestAppContext) {
    let root = fixture("close-resident");
    let (_view, mut cx) = open(app, &root);
    let cancel = inject_running(&mut cx);

    cx.simulate_keystrokes("cmd-w");
    app.run_until_parked();
    assert_eq!(app.update(|cx| cx.windows().len()), 0, "the window closes");
    assert!(!cancel.load(std::sync::atomic::Ordering::Relaxed), "the operation keeps running");
    app.update(|cx| cx.global_mut::<Operations>().running = None);
}

#[gpui::test]
fn quitting_with_no_window_opens_one_and_asks(app: &mut TestAppContext) {
    let root = fixture("quit-resident");
    let (_view, mut cx) = open(app, &root);
    app.update(|cx| cx.set_global(LaunchFolder(root.clone())));
    let cancel = inject_running(&mut cx);

    cx.simulate_keystrokes("cmd-w");
    app.run_until_parked();
    assert_eq!(app.update(|cx| cx.windows().len()), 0);

    app.update(quit);
    app.run_until_parked();
    let handle = app
        .update(|cx| cx.windows().first().and_then(|w| w.downcast::<CrowWindow>()))
        .expect("⌘Q opened a window to ask in");
    let asking =
        handle.update(app, |this, _, _| matches!(this.sheet, Some(Sheet::StopOperation { .. }))).expect("window alive");
    assert!(asking, "the Stop and Quit sheet is up");
    assert!(!cancel.load(std::sync::atomic::Ordering::Relaxed), "nothing is cancelled before the answer");
    app.update(|cx| cx.global_mut::<Operations>().running = None);
}

#[gpui::test]
fn open_path_opens_a_folder_and_reveals_a_file(cx: &mut TestAppContext) {
    let root = fixture("open-path");
    let (_view, mut cx) = open(cx, &root);

    let folder = root.join("alpha");
    cx.update(|_, cx| open_path(folder.clone(), true, cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 2);
    let handle = cx.update(|_, cx| cx.windows()[1].downcast::<CrowWindow>().expect("folder window"));
    let mut folder_cx = VisualTestContext::from_window(handle.into(), &cx);
    let folder_view = handle.root(&mut folder_cx).expect("root view");
    settle(&folder_view, &mut folder_cx);
    assert_eq!(dir(&folder_view, &folder_cx), folder);

    let file = root.join("notes.txt");
    cx.update(|_, cx| open_path(file.clone(), false, cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 3);
    let handle = cx.update(|_, cx| cx.windows()[2].downcast::<CrowWindow>().expect("file window"));
    let mut file_cx = VisualTestContext::from_window(handle.into(), &cx);
    let file_view = handle.root(&mut file_cx).expect("root view");
    settle(&file_view, &mut file_cx);
    assert_eq!(dir(&file_view, &file_cx), root);
    assert_eq!(selected(&file_view, &file_cx).as_deref(), Some("notes.txt"));
}

#[gpui::test]
fn closing_with_other_windows_open_just_closes(app: &mut TestAppContext) {
    let root = fixture("close-free");
    let (_view, mut cx) = open(app, &root);
    cx.simulate_keystrokes("cmd-n");
    cx.run_until_parked();
    assert_eq!(app.update(|cx| cx.windows().len()), 2);
    let _cancel = inject_running(&mut cx);
    // Another window stays open, so the operation keeps going without asking.
    cx.simulate_keystrokes("cmd-w");
    app.run_until_parked();
    assert_eq!(app.update(|cx| cx.windows().len()), 1, "closed without a sheet");
    app.update(|cx| cx.global_mut::<Operations>().running = None);
}

#[gpui::test]
fn quitting_mid_operation_asks_first(cx: &mut TestAppContext) {
    let root = fixture("quit-guard");
    let (view, mut cx) = open(cx, &root);
    let cancel = inject_running(&mut cx);
    // Menu, palette and ⌘Q all go through `quit`.
    cx.update(|_, cx| quit(cx));
    cx.run_until_parked();
    assert!(
        view.read_with(&cx, |w, _| matches!(w.sheet, Some(Sheet::StopOperation { .. }))),
        "⌘Q asks while the move runs"
    );
    cx.simulate_keystrokes("escape");
    assert!(!cancel.load(std::sync::atomic::Ordering::Relaxed));
    cx.update(|_, cx| cx.global_mut::<Operations>().running = None);
}

#[gpui::test]
fn list_name_column_follows_the_window_width(cx: &mut TestAppContext) {
    let root = fixture("list-width");
    let (_view, mut cx) = open(cx, &root);
    let widths = |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        // The pane asks for one more frame once it sees the new width; the test
        // platform does not run scheduled frames, so draw it here.
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let name = cx.debug_bounds("column-header-0").expect("Name header").size.width;
        let next = cx.debug_bounds("column-header-1").expect("second header").size.width;
        let table = cx.debug_bounds("table-header").expect("header").size.width;
        (name, next, table)
    };
    cx.simulate_resize(gpui::size(px(1400.), px(700.)));
    let (wide_name, wide_next, wide_table) = widths(&mut cx);
    cx.simulate_resize(gpui::size(px(1100.), px(700.)));
    let (narrow_name, narrow_next, narrow_table) = widths(&mut cx);
    assert!(wide_table > narrow_table, "the list got narrower");
    assert_eq!(wide_next, narrow_next, "the other columns keep their widths");
    assert_eq!(wide_name - narrow_name, wide_table - narrow_table, "Name absorbs the whole change");

    // Narrower than every column's minimum: whole columns hide from the right, so no
    // cell runs past the list's edge.
    for width in [900., 760., 640.] {
        cx.simulate_resize(gpui::size(px(width), px(700.)));
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let table = cx.debug_bounds("table-header").expect("header");
        let shown: Vec<_> = (0..6).filter_map(|i| cx.debug_bounds(format!("column-header-{i}").leak())).collect();
        assert!(!shown.is_empty(), "Name always shows");
        for cell in &shown {
            assert!(cell.right() <= table.right() + px(0.5), "at {width}: {cell:?} past {table:?}");
        }
    }
}

#[gpui::test]
fn hidden_files_toggle_applies_to_every_tab(cx: &mut TestAppContext) {
    let root = fixture("hidden");
    std::fs::write(root.join(".secret"), "").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    let count = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, cx| p.item_count(cx));
    assert_eq!(count(&cx), 4, "hidden files show by default");

    cx.simulate_keystrokes("cmd-t cmd-shift-.");
    settle(&view, &mut cx);
    assert_eq!(count(&cx), 3);
    cx.simulate_keystrokes("cmd-shift-[");
    assert_eq!(count(&cx), 3, "The other tab hides hidden files too");
}

fn selected_names(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> Vec<String> {
    pane(view, cx).read_with(cx, |p, cx| p.selected_names(cx))
}

fn names(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> Vec<String> {
    pane(view, cx).read_with(cx, |p, cx| p.names(cx))
}

#[gpui::test]
fn shift_arrows_extend_and_cmd_a_selects_all(cx: &mut TestAppContext) {
    let root = fixture("multi");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down shift-down");
    assert_eq!(selected_names(&view, &cx), ["beta", "notes.txt"]);
    cx.simulate_keystrokes("shift-up shift-up");
    assert_eq!(selected_names(&view, &cx), ["alpha", "beta"], "the range pivots on where it started");
    cx.simulate_keystrokes("cmd-a");
    assert_eq!(selected_names(&view, &cx), ["alpha", "beta", "notes.txt"]);
    cx.simulate_keystrokes("down");
    assert_eq!(selected_names(&view, &cx), ["beta"], "a plain arrow collapses to one item");
}

#[gpui::test]
fn typing_selects_by_name_and_space_continues_a_name(cx: &mut TestAppContext) {
    let root = fixture("typeselect");
    std::fs::write(root.join("my file.txt"), "").expect("create fixture");
    std::fs::write(root.join("my_other.txt"), "").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("b e");
    assert_eq!(selected(&view, &cx).as_deref(), Some("beta"));
    std::thread::sleep(crow_core::TypeSelect::TIMEOUT + std::time::Duration::from_millis(50));
    cx.simulate_keystrokes("m y space f");
    assert_eq!(selected(&view, &cx).as_deref(), Some("my file.txt"), "Space mid-name is part of the name");
}

#[gpui::test]
fn filter_narrows_rows_and_escape_restores_focus(cx: &mut TestAppContext) {
    let root = fixture("filter");
    let (view, mut cx) = open(cx, &root);

    cx.simulate_keystrokes("/");
    cx.simulate_input("AL");
    settle(&view, &mut cx);
    assert_eq!(rows(&view, &cx), [("alpha".to_owned(), 0)], "matching is case-insensitive");
    assert_eq!(pane(&view, &cx).read_with(&cx, |p, cx| p.item_count(cx)), 3, "the total stays visible");

    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    assert_eq!(rows(&view, &cx).len(), 3, "Escape clears the filter");
    cx.simulate_keystrokes("n");
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "focus is back on the list for type-to-select");
}

#[gpui::test]
fn filter_exposes_match_ranges_per_row(cx: &mut TestAppContext) {
    let root = fixture("filter-ranges");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("/");
    cx.simulate_input("PH");
    settle(&view, &mut cx);
    assert_eq!(rows(&view, &cx), [("alpha".to_owned(), 0)]);
    assert_eq!(pane(&view, &cx).read_with(&cx, |p, cx| p.row_matches(cx)), [Some(2..4)]);
}

#[gpui::test]
fn filter_arrows_and_return_act_on_matching_rows(cx: &mut TestAppContext) {
    let root = fixture("filter-enter");
    let (view, mut cx) = open(cx, &root);

    cx.simulate_keystrokes("/");
    cx.simulate_input("a");
    settle(&view, &mut cx);
    assert_eq!(rows(&view, &cx).iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["alpha", "beta"]);

    cx.simulate_keystrokes("down");
    assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "arrows move within the matches");
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root, "Return in the field does not open anything");
    assert!(pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()), "the filter stays");
    assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "the list has focus on the selected match");
    let list_focus = pane(&view, &cx).read_with(&cx, |p, _| p.focus_handle().clone());
    let list_focused = cx.update(|window, _| list_focus.is_focused(window));
    assert!(list_focused, "Return moves focus into the matches");
    cx.simulate_keystrokes("cmd-down");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("beta"), "the list's keys open the match");
    assert!(rows(&view, &cx).is_empty(), "the filter does not follow the navigation");

    cx.simulate_keystrokes("cmd-up");
    settle(&view, &mut cx);
    assert_eq!(rows(&view, &cx).len(), 3, "going back shows the whole folder again");
}

#[gpui::test]
fn filter_return_lands_on_the_first_match(cx: &mut TestAppContext) {
    let root = fixture("filter-return-first");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("/");
    cx.simulate_input("b");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "the first match is selected");
    let list_focus = pane(&view, &cx).read_with(&cx, |p, _| p.focus_handle().clone());
    let list_focused = cx.update(|window, _| list_focus.is_focused(window));
    assert!(list_focused, "focus left the field for the list");
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    assert!(!pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()), "Esc in the list clears the filter");

    cx.simulate_keystrokes("/");
    cx.simulate_input("zzz");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    let field = pane(&view, &cx).read_with(&cx, |p, cx| p.filter_focus(cx));
    let field_focused = cx.update(|window, _| field.is_some_and(|focus| focus.is_focused(window)));
    assert!(field_focused, "with no matches the field keeps focus");
}

#[gpui::test]
fn filter_commands_and_escape_work_with_the_list_focused(cx: &mut TestAppContext) {
    let root = fixture("filter-list-focus");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("/");
    cx.simulate_input("a");
    settle(&view, &mut cx);

    let focus = pane(&view, &cx).read_with(&cx, |p, _| p.focus_handle().clone());
    cx.update(|window, cx| window.focus(&focus, cx));
    cx.simulate_keystrokes("cmd-a");
    assert_eq!(selected_names(&view, &cx), ["alpha", "beta"], "Cmd-A selects matching rows only");

    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    assert_eq!(rows(&view, &cx).len(), 3, "Escape on the list clears the filter too");
}

#[gpui::test]
fn filter_ignores_inline_children_until_cleared(cx: &mut TestAppContext) {
    let root = fixture("filter-inline");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("right");
    settle(&view, &mut cx);
    assert!(rows(&view, &cx).contains(&("inner".to_owned(), 1)), "alpha is open inline");

    cx.simulate_keystrokes("/");
    cx.simulate_input("inner");
    settle(&view, &mut cx);
    assert!(rows(&view, &cx).is_empty(), "a filter shows this folder's own matches, flat");

    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    assert!(rows(&view, &cx).contains(&("inner".to_owned(), 1)), "clearing restores the open folder");
}

#[gpui::test]
fn filter_letters_stay_in_the_field_in_the_vim_layer(cx: &mut TestAppContext) {
    let root = fixture("filter-vim");
    let (view, mut cx) = open(cx, &root);
    use_vim(&view, &mut cx);

    cx.simulate_keystrokes("/");
    cx.simulate_input("al");
    settle(&view, &mut cx);
    assert_eq!(rows(&view, &cx), [("alpha".to_owned(), 0)], "l filters instead of moving right");

    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("j");
    assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "Vim keys work again after the filter");
}

#[gpui::test]
fn filter_narrows_the_focused_column(cx: &mut TestAppContext) {
    let root = fixture("filter-columns");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3");
    settle(&view, &mut cx);

    cx.simulate_keystrokes("/");
    cx.simulate_input("al");
    settle(&view, &mut cx);
    assert_eq!(rows(&view, &cx), [("alpha".to_owned(), 0)]);

    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    assert_eq!(rows(&view, &cx).len(), 3);
}

#[gpui::test]
fn filter_field_opens_in_the_focused_column(cx: &mut TestAppContext) {
    let root = fixture("filter-field-column");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3 right");
    settle(&view, &mut cx);
    let before0 = cx.debug_bounds("column-0").expect("first column");
    let before1 = cx.debug_bounds("column-1").expect("second column");
    let count_before = pane(&view, &cx).read_with(&cx, |p, _| p.folder_count());
    assert_eq!(pane(&view, &cx).read_with(&cx, |p, _| p.focused_index()), 1, "moved into the second column");

    cx.simulate_keystrokes("/");
    settle(&view, &mut cx);
    let field = cx.debug_bounds("filter-field").expect("the field is in the column");
    let after0 = cx.debug_bounds("column-0").expect("first column");
    let after1 = cx.debug_bounds("column-1").expect("second column");
    assert!(field.left() >= after1.left() && field.right() <= after1.right(), "{field:?} inside {after1:?}");
    assert_eq!(after0, before0, "the column to the left did not move");
    assert_eq!(after0.size.width, before0.size.width, "widths do not change while filtering");
    assert_eq!(after1, before1, "the filtered column does not move or grow");
    assert_eq!(pane(&view, &cx).read_with(&cx, |p, _| p.folder_count()), count_before, "no column opened");
    assert_eq!(pane(&view, &cx).read_with(&cx, |p, _| p.focused_index()), 1);
}

#[gpui::test]
fn filter_field_opens_above_the_list_rows(cx: &mut TestAppContext) {
    let root = fixture("filter-field-list");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("/");
    settle(&view, &mut cx);
    let header = cx.debug_bounds("table-header").expect("header");
    let field = cx.debug_bounds("filter-field").expect("field");
    let row = cx.debug_bounds("row-alpha").expect("first row");
    assert!(field.top() >= header.bottom(), "{field:?} is below the {header:?}");
    assert!(field.bottom() <= row.top(), "{field:?} is above the {row:?}");
}

#[gpui::test]
fn filter_vim_letters_stay_in_the_column_field(cx: &mut TestAppContext) {
    let root = fixture("filter-vim-column-field");
    let (view, mut cx) = open(cx, &root);
    use_vim(&view, &mut cx);
    cx.simulate_keystrokes("cmd-3 right");
    settle(&view, &mut cx);
    let before = (dir(&view, &cx), pane(&view, &cx).read_with(&cx, |p, _| p.focused_index()));
    cx.simulate_keystrokes("/");
    cx.simulate_keystrokes("h l -");
    assert_eq!(pane(&view, &cx).read_with(&cx, |p, cx| p.filter_query(cx)).as_deref(), Some("hl-"));
    assert_eq!((dir(&view, &cx), pane(&view, &cx).read_with(&cx, |p, _| p.focused_index())), before);
    assert!(rows(&view, &cx).is_empty(), "no Vim motion or filter match ran");
}

#[gpui::test]
fn filter_field_takes_space_slash_and_list_keys_as_text(cx: &mut TestAppContext) {
    let root = fixture("filter-field-keys");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("/");
    cx.simulate_keystrokes("a space /");
    assert_eq!(pane(&view, &cx).read_with(&cx, |p, cx| p.filter_query(cx)).as_deref(), Some("a /"));
    cx.simulate_keystrokes("cmd-backspace");
    assert_eq!(
        pane(&view, &cx).read_with(&cx, |p, cx| p.filter_query(cx)).as_deref(),
        Some(""),
        "Cmd-Backspace deletes to the start of the field"
    );
    assert!(root.join("notes.txt").exists(), "Cmd-Backspace did not trash a row from the field");
}

#[gpui::test]
fn click_in_a_scrolled_filter_field_lands_on_the_visible_text(cx: &mut TestAppContext) {
    let root = fixture("filter-scrolled-click");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("/");
    // Far wider than the column, so the field scrolls to keep the caret at the end.
    let long = "abcdefghij".repeat(12);
    cx.simulate_input(&long);
    cx.run_until_parked();
    let field = cx.debug_bounds("filter-field").expect("field on screen");
    // Just right of the accent `/`: the start of the visible text, not of the query.
    cx.simulate_click(gpui::point(field.left() + px(30.), field.center().y), gpui::Modifiers::default());
    cx.simulate_input("X");
    let query = pane(&view, &cx).read_with(&cx, |p, cx| p.filter_query(cx)).expect("filtering");
    let at = query.find('X').expect("typed");
    assert!(at > 20, "the click landed on the scrolled-away start (X at {at}): {query}");
}

#[gpui::test]
fn filter_closes_when_the_focused_column_changes(cx: &mut TestAppContext) {
    let root = fixture("filter-column-change");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3 right");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("/");
    cx.simulate_input("x");
    settle(&view, &mut cx);
    assert!(pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()));

    // The field owns Left while it has focus; click the column's list first.
    let focus = pane(&view, &cx).read_with(&cx, |p, _| p.focus_handle().clone());
    cx.update(|window, cx| window.focus(&focus, cx));
    cx.simulate_keystrokes("left");
    settle(&view, &mut cx);
    assert!(!pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()));
    assert!(cx.debug_bounds("filter-field").is_none(), "the field closed");
    assert_eq!(rows(&view, &cx).len(), 3, "the left column shows all of its rows");
}

#[gpui::test]
fn filter_follows_live_changes(cx: &mut TestAppContext) {
    let root = fixture("filter-live");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("/");
    cx.simulate_input("al");
    settle(&view, &mut cx);
    std::fs::write(root.join("alpine.txt"), "").expect("create fixture");
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        // The folder's watcher polls every 100 ms on the test clock.
        cx.executor().advance_clock(std::time::Duration::from_millis(150));
        cx.run_until_parked();
        if rows(&view, &cx).iter().any(|(name, _)| name == "alpine.txt") {
            break;
        }
    }
    assert_eq!(rows(&view, &cx).iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["alpha", "alpine.txt"]);
    let matches = pane(&view, &cx).read_with(&cx, |p, cx| p.row_matches(cx));
    assert_eq!(matches, [Some(0..2), Some(0..2)], "both rows carry the live match range");
}

#[gpui::test]
fn cmd_f_while_filtering_searches_the_focused_folder_with_the_query(cx: &mut TestAppContext) {
    let root = fixture("filter-to-search");
    std::fs::write(root.join("alpha/needle.txt"), "nothing\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3 right");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("/");
    cx.simulate_input("nee");
    settle(&view, &mut cx);
    assert!(pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()));

    cx.simulate_keystrokes("cmd-f");
    wait_for_search(&view, &mut cx);
    let search = view.read_with(&cx, |w, _| w.search.clone()).expect("search opens");
    assert_eq!(search.read_with(&cx, |search, cx| search.query_text(cx)), "nee");
    assert_eq!(search.read_with(&cx, |search, _| search.folder().to_path_buf()), root.join("alpha"));
    assert_eq!(search.read_with(&cx, |search, _| search.scope()), crate::search::Scope::Folder);
    let paths = search.read_with(&cx, |search, _| search.result_paths());
    assert!(paths.contains(&root.join("alpha/needle.txt")), "{paths:?}");
    assert!(!pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()), "the filter was handed over, not left open");

    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    assert_eq!(rows(&view, &cx).len(), 2, "the focused column shows all of its rows again");
    cx.simulate_keystrokes("n");
    assert_eq!(selected(&view, &cx).as_deref(), Some("needle.txt"), "type-to-select proves the list has focus");
}

#[gpui::test]
fn cmd_f_from_the_list_while_filtering_also_escalates(cx: &mut TestAppContext) {
    let root = fixture("filter-list-to-search");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("/");
    cx.simulate_input("a");
    settle(&view, &mut cx);
    let focus = pane(&view, &cx).read_with(&cx, |p, _| p.focus_handle().clone());
    cx.update(|window, cx| window.focus(&focus, cx));

    cx.simulate_keystrokes("cmd-f");
    wait_for_search(&view, &mut cx);
    let query = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search").read(cx).query_text(cx));
    assert_eq!(query, "a");
    assert!(!pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()));
}

#[gpui::test]
fn filter_footer_click_opens_search(cx: &mut TestAppContext) {
    let root = fixture("filter-footer-click");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("/");
    cx.simulate_input("al");
    settle(&view, &mut cx);
    click_selector(&mut cx, "filter-search-footer");
    wait_for_search(&view, &mut cx);
    let query = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search").read(cx).query_text(cx));
    assert_eq!(query, "al");
}

#[gpui::test]
fn filter_with_no_matches_offers_search(cx: &mut TestAppContext) {
    let root = fixture("filter-no-matches");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("/");
    cx.simulate_input("zzz");
    settle(&view, &mut cx);
    assert!(cx.debug_bounds("filter-no-matches").is_some(), "D replaces the list");
    assert!(cx.debug_bounds("filter-search-footer").is_none(), "there is no footer in D");

    click_selector(&mut cx, "filter-search-subfolders");
    wait_for_search(&view, &mut cx);
    let query = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search").read(cx).query_text(cx));
    assert_eq!(query, "zzz");
    assert!(!pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()));
}

#[gpui::test]
fn filter_no_match_note_follows_hidden_files(cx: &mut TestAppContext) {
    let root = fixture("filter-no-matches-hidden");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("/");
    cx.simulate_input("zzz");
    settle(&view, &mut cx);
    assert!(cx.debug_bounds("filter-hidden-note").is_some(), "hidden files are off");
    cx.simulate_keystrokes("cmd-shift-.");
    settle(&view, &mut cx);
    assert!(cx.debug_bounds("filter-hidden-note").is_some(), "the way out is still named");
    assert!(cx.debug_bounds("filter-no-matches").is_some());
}

#[gpui::test]
fn filter_footer_hides_for_an_empty_query(cx: &mut TestAppContext) {
    let root = fixture("filter-footer-empty");
    let (_view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("/");
    settle(&_view, &mut cx);
    assert!(cx.debug_bounds("filter-field").is_some(), "the field is open");
    assert!(cx.debug_bounds("filter-search-footer").is_none(), "nothing to carry deeper");
}

#[gpui::test]
fn sorting_keeps_folders_first_and_the_selection(cx: &mut TestAppContext) {
    let root = fixture("sort");
    std::fs::write(root.join("big.bin"), vec![0u8; 5000]).expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down down");
    let picked = selected(&view, &cx);
    pane(&view, &cx).update(&mut cx, |p, cx| p.set_sort(crow_fs::Sort::default().toggled(crow_fs::SortKey::Size), cx));
    settle(&view, &mut cx);
    assert_eq!(names(&view, &cx), ["alpha", "beta", "big.bin", "notes.txt"], "largest file first, folders on top");
    assert_eq!(selected(&view, &cx), picked, "the selection follows the item, not the row");
}

#[gpui::test]
fn sorting_while_a_folder_loads_keeps_its_listing(cx: &mut TestAppContext) {
    let root = fixture("sort-while-loading");
    let (view, mut cx) = open(cx, &root);
    // Enter alpha: its new Folder starts loading with no rows yet.
    view.update(&mut cx, |window, cx| {
        let dir = window.pane().read(cx).current_dir(cx).join("alpha");
        window.pane().update(cx, |pane, cx| pane.navigate(dir, cx));
    });
    let folder = pane(&view, &cx).read_with(&cx, |pane, _| pane.test_folder());
    assert!(folder.read_with(&cx, |folder, _| folder.is_loading()), "the new folder is still loading");
    assert!(names(&view, &cx).is_empty(), "no rows before the load");
    // Hold the re-sort in flight until after the load has landed. Before the fix,
    // the sort applied the empty listing captured before the load and wiped it.
    folder.update(&mut cx, |folder, _| folder.set_sort_delay(Duration::from_millis(100)));
    pane(&view, &cx)
        .update(&mut cx, |pane, cx| pane.set_sort(crow_fs::Sort::default().toggled(crow_fs::SortKey::Size), cx));
    settle(&view, &mut cx);
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(names(&view, &cx), ["inner"], "the load that finished during the sort survives");
}

/// H4 #9: a re-sort computed from an old listing must not overwrite a newer one.
/// The guard re-sorts the listing that replaced it instead of applying its result.
#[gpui::test]
fn a_stale_resort_keeps_a_newer_listing(cx: &mut TestAppContext) {
    let root = fixture("stale-resort");
    std::fs::write(root.join(".hidden"), "x").expect("write hidden");
    let (view, mut cx) = open(cx, &root);
    let folder = pane(&view, &cx).read_with(&cx, |pane, _| pane.test_folder());
    folder.update(&mut cx, |folder, _| folder.set_sort_delay(Duration::from_millis(100)));
    // A sort on the loaded folder captures the listing with hidden files.
    pane(&view, &cx)
        .update(&mut cx, |pane, cx| pane.set_sort(crow_fs::Sort::default().toggled(crow_fs::SortKey::Size), cx));
    // Hiding files loads a listing without them while the sort is in flight.
    folder.update(&mut cx, |folder, cx| folder.set_show_hidden(false, cx));
    settle(&view, &mut cx);
    folder.update(&mut cx, |folder, _| folder.set_sort_delay(Duration::ZERO));
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    let names = names(&view, &cx);
    assert!(!names.contains(&".hidden".to_owned()), "the newer listing survives: {names:?}");
}

#[gpui::test]
fn exclude_config_reloads_live_and_keeps_the_last_good_rules(cx: &mut TestAppContext) {
    let root = fixture("exclude-reload");
    let (view, mut cx) = open(cx, &root);
    let names = |cx: &VisualTestContext| cx.read(|cx| cx.global::<Ui>().exclude.set(crow_core::SetName::Dev).names);
    assert!(names(&cx).contains(&"node_modules".to_owned()));

    write_config(&view, &mut cx, "exclude.toml", "[dev]\nnames = [\"only_this\"]\n");
    assert_eq!(names(&cx), vec!["only_this".to_owned()]);

    write_config(&view, &mut cx, "exclude.toml", "this is not toml");
    assert_eq!(names(&cx), vec!["only_this".to_owned()], "a broken file keeps the last good rules");
    let error = cx.read(|cx| cx.global::<Ui>().config_error().map(|e| e.to_string()).unwrap_or_default());
    assert!(error.contains("exclude.toml"), "{error}");
}

#[gpui::test]
fn clicking_the_scrim_closes_every_overlay(cx: &mut TestAppContext) {
    theme_index();
    let root = fixture("scrim");
    let (view, mut cx) = open(cx, &root);
    let start_theme = cx.read(|cx| cx.global::<Ui>().theme.name.clone());
    let pane_focus = view.read_with(&cx, |w, cx| w.pane().read(cx).focus_handle().clone());
    let click_scrim = |cx: &mut VisualTestContext| {
        let bounds = cx.debug_bounds("sheet-scrim").expect("the scrim is laid out");
        cx.simulate_click(gpui::point(bounds.origin.x + px(10.), bounds.bottom() - px(10.)), Default::default());
        settle(&view, cx);
    };
    let focused = |cx: &mut VisualTestContext| cx.update(|window, _| pane_focus.is_focused(window));

    cx.simulate_keystrokes("cmd-shift-g");
    settle(&view, &mut cx);
    assert!(view.read_with(&cx, |w, _| w.palette.is_some()), "Go to Folder (the palette) is open");
    click_scrim(&mut cx);
    assert!(view.read_with(&cx, |w, _| w.palette.is_none()), "a scrim click closes it like Escape");
    assert!(focused(&mut cx), "focus goes back to the pane");
    assert!(root.join("notes.txt").exists());

    cx.simulate_keystrokes("down down cmd-alt-backspace");
    settle(&view, &mut cx);
    assert!(sheet_open(&view, &cx), "the delete sheet is open");
    click_scrim(&mut cx);
    assert!(!sheet_open(&view, &cx), "a scrim click cancels it");
    assert!(root.join("notes.txt").exists(), "cancel runs nothing");
    assert!(focused(&mut cx));

    cx.simulate_keystrokes("cmd-j");
    settle(&view, &mut cx);
    assert!(view.read_with(&cx, |w, _| w.why.is_some()), "Why This View? is open");
    click_scrim(&mut cx);
    assert!(view.read_with(&cx, |w, _| w.why.is_none()));
    assert!(focused(&mut cx));

    cx.simulate_keystrokes("cmd-alt-t");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("down");
    assert!(view.read_with(&cx, |w, _| w.picker.is_some()), "the theme picker is open");
    assert_ne!(cx.read(|cx| cx.global::<Ui>().theme.name.clone()), start_theme, "a theme is previewed");
    click_scrim(&mut cx);
    assert!(view.read_with(&cx, |w, _| w.picker.is_none()));
    assert_eq!(cx.read(|cx| cx.global::<Ui>().theme.name.clone()), start_theme, "the preview is reverted");
    assert!(focused(&mut cx));

    cx.simulate_keystrokes("cmd-k");
    settle(&view, &mut cx);
    assert!(view.read_with(&cx, |w, _| w.palette.is_some()), "the palette is open");
    click_scrim(&mut cx);
    assert!(view.read_with(&cx, |w, _| w.palette.is_none()));
    assert!(focused(&mut cx));
}

#[gpui::test]
fn clicking_inside_a_sheet_keeps_it_open(cx: &mut TestAppContext) {
    let root = fixture("sheet-inside");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-j");
    settle(&view, &mut cx);
    let panel = cx.debug_bounds("sheet-panel").expect("the panel is laid out");
    cx.simulate_click(panel.center(), Default::default());
    settle(&view, &mut cx);
    assert!(view.read_with(&cx, |w, _| w.why.is_some()), "a click inside does not dismiss");

    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("down down cmd-alt-backspace");
    settle(&view, &mut cx);
    assert!(sheet_open(&view, &cx));
    let cancel = cx.debug_bounds("sheet-cancel").expect("the Cancel button is laid out");
    cx.simulate_click(cancel.center(), Default::default());
    settle(&view, &mut cx);
    assert!(!sheet_open(&view, &cx), "a button inside still runs its own click");
    assert!(root.join("notes.txt").exists());
}

#[gpui::test]
fn go_to_folder_opens_the_palette_on_this_folder(cx: &mut TestAppContext) {
    let root = fixture("goto");
    let (view, mut cx) = open(cx, &root);
    let typed = |cx: &VisualTestContext| {
        view.read_with(cx, |w, cx| w.palette.as_ref().map(|p| p.input.read(cx).text().to_owned()))
    };

    cx.simulate_keystrokes("cmd-shift-g");
    settle(&view, &mut cx);
    assert_eq!(typed(&cx), Some(format!("{}/", root.display())), "starts on this folder's path");
    cx.simulate_input("al");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("tab");
    settle(&view, &mut cx);
    assert!(typed(&cx).is_some_and(|t| t.ends_with("/alpha/")), "Tab completes a folder: {:?}", typed(&cx));
    cx.simulate_input("inner");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha/inner"));
    assert!(typed(&cx).is_none(), "Return closes the palette");

    cx.simulate_keystrokes("cmd-shift-g cmd-a");
    cx.simulate_input(&root.join("notes.txt").display().to_string());
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root, "a file opens its folder…");
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "…with the file selected");

    cx.simulate_keystrokes("cmd-shift-g cmd-a");
    cx.simulate_input(&root.join("nothing-here").display().to_string());
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert!(typed(&cx).is_some(), "a missing path keeps the palette open");
    cx.simulate_keystrokes("escape");
    assert!(typed(&cx).is_none(), "Escape closes it");
    assert_eq!(dir(&view, &cx), root, "…without going anywhere");
}

/// The bar's path field while ⌘L edits it.
fn path_edit_text(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> Option<String> {
    view.read_with(cx, |w, cx| w.path_edit.as_ref().map(|edit| edit.input.read(cx).text().to_owned()))
}

fn double_click(cx: &mut VisualTestContext, at: gpui::Point<gpui::Pixels>) {
    cx.simulate_event(gpui::MouseDownEvent {
        position: at,
        modifiers: Default::default(),
        button: gpui::MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position: at,
        modifiers: Default::default(),
        button: gpui::MouseButton::Left,
        click_count: 2,
    });
}

#[gpui::test]
fn path_bar_edits_the_path_with_cmd_l(cx: &mut TestAppContext) {
    let root = fixture("path-edit");
    let (view, mut cx) = open(cx, &root);
    let slot_height = cx.debug_bounds("bar-path").expect("path slot").size.height;

    cx.simulate_keystrokes("cmd-l");
    settle(&view, &mut cx);
    assert_eq!(
        path_edit_text(&view, &cx),
        Some(root.join("alpha").display().to_string()),
        "the cursor's path, all selected"
    );
    assert_eq!(cx.debug_bounds("path-bar").expect("bar").size.height, px(30.), "the bar keeps its height");
    assert_eq!(
        cx.debug_bounds("bar-path").expect("path slot").size.height,
        slot_height,
        "the path slot keeps its height"
    );
    assert_eq!(cx.debug_bounds("bar-path-field").expect("field").size.height, px(24.), "the field fits inside it");
    let state = bar_state(&view, &mut cx);
    assert_eq!(hint_labels(&state), ["go", "complete", "cancel"]);
    assert_eq!(hint_keys(&state), ["↩", "⇥", "esc"]);

    // Typing replaces the selection; Tab completes from disk like ⌘K.
    cx.simulate_input(&format!("{}/al", root.display()));
    settle(&view, &mut cx);
    cx.simulate_keystrokes("tab");
    settle(&view, &mut cx);
    assert_eq!(path_edit_text(&view, &cx), Some(format!("{}/alpha/", root.display())));

    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"));
    assert_eq!(path_edit_text(&view, &cx), None, "Return closes the field");
}

#[gpui::test]
fn path_bar_return_reveals_files_and_refuses_missing_paths(cx: &mut TestAppContext) {
    let root = fixture("path-edit-return");
    let (view, mut cx) = open(cx, &root);

    cx.simulate_keystrokes("cmd-l");
    cx.simulate_input(&root.join("alpha").display().to_string());
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"), "Return goes to a folder");

    cx.simulate_keystrokes("cmd-l");
    cx.simulate_input(&root.join("notes.txt").display().to_string());
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root, "a file opens its folder");
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "with the file selected");

    // The bar shows the cursor's path, so the field starts on the file.
    cx.simulate_keystrokes("cmd-l");
    settle(&view, &mut cx);
    assert_eq!(
        path_edit_text(&view, &cx),
        Some(root.join("notes.txt").display().to_string()),
        "the field starts on the shown file"
    );

    // A missing path: beep, keep the field, say why in the bar.
    cx.simulate_input(&root.join("nothing-here").display().to_string());
    settle(&view, &mut cx);
    effect_calls(&mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert!(path_edit_text(&view, &cx).is_some(), "a missing path keeps the field open");
    assert_eq!(dir(&view, &cx), root, "…and goes nowhere");
    let calls = effect_calls(&mut cx);
    assert!(calls.iter().any(|call| matches!(call, crate::effects::Call::Beep)), "the refusal beeps: {calls:?}");
    let state = bar_state(&view, &mut cx);
    let error = state.error.expect("the reason shows in the bar");
    let text = error.text.to_string();
    assert!(text.contains("nothing-here"), "the bar names the path: {text}");

    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    assert_eq!(path_edit_text(&view, &cx), None, "Escape closes the field");
    assert_eq!(dir(&view, &cx), root, "…without navigating");
    let state = bar_state(&view, &mut cx);
    assert!(state.error.is_none(), "the refusal goes with the field");
}

#[gpui::test]
fn path_edit_starts_on_the_shown_search_result(cx: &mut TestAppContext) {
    let root = fixture("path-edit-search");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-f");
    settle(&view, &mut cx);
    cx.simulate_input("notes");
    wait_for_search(&view, &mut cx);
    let selected = view.read_with(&cx, |w, cx| {
        w.search.as_ref().and_then(|search| search.read(cx).selected_entry()).map(|(path, _)| path)
    });
    assert_eq!(selected, Some(root.join("notes.txt")), "the search selects the name hit");

    cx.simulate_keystrokes("cmd-l");
    settle(&view, &mut cx);
    assert_eq!(
        path_edit_text(&view, &cx),
        Some(root.join("notes.txt").display().to_string()),
        "the field starts on the shown result"
    );
}

#[gpui::test]
fn path_bar_double_click_edits_and_single_click_does_not(cx: &mut TestAppContext) {
    let root = fixture("path-edit-click");
    let (view, mut cx) = open(cx, &root);
    let bar = cx.debug_bounds("bar-path").expect("path slot");
    let at = gpui::point(bar.right() - px(8.), bar.center().y);

    cx.simulate_click(at, Default::default());
    settle(&view, &mut cx);
    assert_eq!(path_edit_text(&view, &cx), None, "a single click does nothing");

    double_click(&mut cx, at);
    settle(&view, &mut cx);
    assert_eq!(
        path_edit_text(&view, &cx),
        Some(root.join("alpha").display().to_string()),
        "a double-click edits the shown path"
    );

    // A click on a segment still navigates and never opens the field.
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    let segment = cx.debug_bounds("path-segment-0").expect("first segment");
    double_click(&mut cx, segment.center());
    settle(&view, &mut cx);
    assert_eq!(path_edit_text(&view, &cx), None, "a segment is not empty space");
}

#[gpui::test]
fn path_bar_escape_restores_focus_and_clicking_a_row_cancels(cx: &mut TestAppContext) {
    let root = fixture("path-edit-leave");
    let (view, mut cx) = open(cx, &root);
    // Focus-out listeners only fire for an active window, as in the app.
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();

    cx.simulate_keystrokes("cmd-l");
    settle(&view, &mut cx);
    let input_focus =
        view.read_with(&cx, |w, cx| w.path_edit.as_ref().expect("editing").input.read(cx).focus_handle(cx));
    let focused = cx.update(|window, cx| window.focused(cx));
    assert_eq!(focused, Some(input_focus), "⌘L focuses the field");
    cx.simulate_input(&format!("{}/alpha", root.display()));
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    assert_eq!(path_edit_text(&view, &cx), None);
    assert_eq!(dir(&view, &cx), root, "Escape does not navigate");
    let focused = cx.update(|window, cx| window.focused(cx));
    let pane_focus = pane(&view, &cx).read_with(&cx, |pane, _| pane.focus_handle().clone());
    assert_eq!(focused, Some(pane_focus), "Escape returns focus to the list");

    cx.simulate_keystrokes("cmd-l");
    settle(&view, &mut cx);
    let row = cx.debug_bounds("row-alpha").expect("alpha row");
    cx.simulate_click(row.center(), Default::default());
    settle(&view, &mut cx);
    let focused = cx.update(|window, cx| window.focused(cx));
    let pane_focus = pane(&view, &cx).read_with(&cx, |pane, _| pane.focus_handle().clone());
    assert_eq!(focused, Some(pane_focus), "the row click focuses the list");
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "…and selects the row");
    assert_eq!(path_edit_text(&view, &cx), None, "clicking a row cancels the edit");
}

#[gpui::test]
fn palette_offers_edit_path(cx: &mut TestAppContext) {
    let root = fixture("path-edit-palette");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">edit path");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(
        path_edit_text(&view, &cx),
        Some(root.join("alpha").display().to_string()),
        "the palette command edits the shown path"
    );
}

/// (number of columns, focused column, folder shown in column `k`, cursor name in column `k`).
fn columns(
    view: &Entity<CrowWindow>,
    cx: &VisualTestContext,
    k: usize,
) -> (usize, usize, Option<PathBuf>, Option<String>) {
    pane(view, cx)
        .read_with(cx, |p, cx| (p.folder_count(), p.focused_index(), p.folder_dir(k, cx), p.cursor_name_in(k, cx)))
}

#[gpui::test]
fn column_view_follows_the_focus_rules(cx: &mut TestAppContext) {
    let root = fixture("columns");
    std::fs::write(root.join("alpha/zeta.txt"), "z").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3");
    settle(&view, &mut cx);
    let (count, focused, child, child_cursor) = columns(&view, &cx, 1);
    assert_eq!((count, focused), (2, 0), "the selected folder's contents show in a second column");
    assert_eq!(child, Some(root.join("alpha")));
    assert_eq!(child_cursor, None, "…which is not focused and has nothing selected");

    cx.simulate_keystrokes("down");
    settle(&view, &mut cx);
    assert_eq!(columns(&view, &cx, 1).2, Some(root.join("beta")), "the child column follows the selection");
    cx.simulate_keystrokes("down");
    settle(&view, &mut cx);
    assert_eq!(columns(&view, &cx, 0).0, 1, "a file ends the chain (its preview is the last column)");

    cx.simulate_keystrokes("up up right");
    settle(&view, &mut cx);
    let (count, focused, _, cursor) = columns(&view, &cx, 1);
    assert_eq!(focused, 1, "Right moves into the child column");
    assert_eq!(cursor.as_deref(), Some("inner"), "…selecting its first item");
    assert_eq!(count, 3, "…whose own contents open to the right");

    cx.simulate_keystrokes("z");
    assert_eq!(columns(&view, &cx, 1).3.as_deref(), Some("zeta.txt"), "typing selects in the focused column");
    assert_eq!(columns(&view, &cx, 0).3.as_deref(), Some("alpha"), "…and leaves the parent alone");
    assert_eq!(dir(&view, &cx), root.join("alpha"), "the window shows the focused column's folder");

    cx.simulate_keystrokes("left");
    let (count, focused, _, cursor) = columns(&view, &cx, 1);
    assert_eq!((count, focused), (2, 0), "Left moves back; the child column stays");
    assert_eq!(cursor, None, "…with its selection cleared");
}

#[gpui::test]
fn column_view_left_on_the_leftmost_column_goes_up(cx: &mut TestAppContext) {
    let root = fixture("columns-left");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3");
    settle(&view, &mut cx);
    assert_eq!(columns(&view, &cx, 0).2, Some(root.clone()));

    let parent = root.parent().expect("the fixture has a parent").to_path_buf();
    cx.simulate_keystrokes("left");
    settle(&view, &mut cx);
    let (_, focused, first, cursor) = columns(&view, &cx, 0);
    assert_eq!(focused, 0);
    assert_eq!(first, Some(parent.clone()), "the parent is pushed in from the left");
    assert_eq!(cursor.as_deref(), Some(root.file_name().unwrap().to_str().unwrap()), "…with the old folder selected");
    assert_eq!(pane(&view, &cx).read_with(&cx, |p, _| p.mode()), ViewMode::Columns, "and column view stays on");
    assert_eq!(dir(&view, &cx), parent);

    cx.simulate_keystrokes("cmd-[");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root, "Back returns to where the columns started");

    // At the root of the disk there is nowhere to go up to.
    pane(&view, &cx).update(&mut cx, |p, cx| p.navigate(PathBuf::from("/"), cx));
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-3");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("left");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), PathBuf::from("/"), "Left at / does nothing");
}

#[gpui::test]
fn list_view_expands_folders_inline(cx: &mut TestAppContext) {
    let root = fixture("expand");
    std::fs::write(root.join("alpha/a.txt"), "").expect("create fixture");
    std::fs::write(root.join("alpha/b.txt"), "").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    assert_eq!(rows(&view, &cx), [("alpha".to_owned(), 0), ("beta".to_owned(), 0), ("notes.txt".to_owned(), 0)]);

    cx.simulate_keystrokes("right");
    settle(&view, &mut cx);
    assert_eq!(
        rows(&view, &cx),
        [
            ("alpha".to_owned(), 0),
            ("inner".to_owned(), 1),
            ("a.txt".to_owned(), 1),
            ("b.txt".to_owned(), 1),
            ("beta".to_owned(), 0),
            ("notes.txt".to_owned(), 0),
        ],
        "Right opens the folder inline, indented under it"
    );
    assert_eq!(dir(&view, &cx), root, "…without going anywhere");
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "the folder stays selected");

    cx.simulate_keystrokes("down");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("right");
    settle(&view, &mut cx);
    assert_eq!(selected(&view, &cx).as_deref(), Some("inner"), "Right expands a nested folder");
    cx.simulate_keystrokes("right");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha/inner"), "Right again goes into the open folder");

    cx.simulate_keystrokes("cmd-[");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root);
    assert!(!rows(&view, &cx).iter().any(|(_, depth)| *depth > 0), "coming back starts collapsed");
}

#[gpui::test]
fn list_view_left_steps_to_the_parent_row_then_collapses(cx: &mut TestAppContext) {
    let root = fixture("left-parent");
    std::fs::write(root.join("alpha/a.txt"), "").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("right");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("down");
    settle(&view, &mut cx);
    assert_eq!(selected(&view, &cx).as_deref(), Some("inner"), "a nested row is selected");

    cx.simulate_keystrokes("left");
    settle(&view, &mut cx);
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "Left selects the row that contains it");
    assert!(rows(&view, &cx).iter().any(|(name, _)| name == "inner"), "…and leaves it open");

    cx.simulate_keystrokes("left");
    settle(&view, &mut cx);
    assert!(!rows(&view, &cx).iter().any(|(name, _)| name == "inner"), "Left again collapses it");

    cx.simulate_keystrokes("down down");
    settle(&view, &mut cx);
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"));
    cx.simulate_keystrokes("left");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.parent().unwrap(), "Left on a top-level row goes up a folder");
    assert_eq!(selected(&view, &cx).as_deref(), Some(root.file_name().unwrap().to_str().unwrap()));
}

#[gpui::test]
fn selection_and_trash_span_expanded_levels(cx: &mut TestAppContext) {
    let root = fixture("expand-select");
    let (a, b) = (root.join("alpha/a.txt"), root.join("alpha/b.txt"));
    std::fs::write(&a, "").expect("create fixture");
    std::fs::write(&b, "").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("right");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("down down");
    settle(&view, &mut cx);
    assert_eq!(selected(&view, &cx).as_deref(), Some("a.txt"));
    cx.simulate_keystrokes("shift-down");
    let selected_paths = pane(&view, &cx).read_with(&cx, |p, cx| p.selected_paths(cx));
    assert_eq!(selected_paths, [a.clone(), b.clone()], "a range over rows of the expanded folder");

    cx.simulate_keystrokes("cmd-backspace");
    settle(&view, &mut cx);
    assert!(!a.exists() && !b.exists(), "both nested files went to the trash");
    assert!(root.join("alpha").is_dir(), "their folder stays");
}

#[gpui::test]
fn switching_to_list_view_keeps_the_folder_you_are_in(cx: &mut TestAppContext) {
    let root = fixture("switch");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("right");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-2");
    settle(&view, &mut cx);
    assert_eq!(pane(&view, &cx).read_with(&cx, |p, _| p.mode()), ViewMode::List);
    assert_eq!(dir(&view, &cx), root.join("alpha"));
    assert_eq!(selected(&view, &cx).as_deref(), Some("inner"), "the selection comes along");
    cx.simulate_keystrokes("cmd-[");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root, "and Back returns to where the columns started");
}

fn sheet_open(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> bool {
    view.read_with(cx, |w, _| w.sheet.is_some())
}

#[gpui::test]
fn new_folder_starts_a_rename_and_return_applies_it(cx: &mut TestAppContext) {
    let root = fixture("newfolder");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-shift-n");
    settle(&view, &mut cx);
    cx.run_until_parked();
    let renaming = pane(&view, &cx).read_with(&cx, |p, cx| p.rename_text(cx));
    assert_eq!(renaming.as_deref(), Some("untitled folder"), "the new folder is created and its rename starts");
    cx.simulate_input("Projects");
    let typed = pane(&view, &cx).read_with(&cx, |p, cx| p.rename_text(cx));
    assert_eq!(typed.as_deref(), Some("Projects"), "the whole name was selected, so typing replaced it");
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    let entries = std::fs::read_dir(&root).unwrap().filter_map(Result::ok).map(|e| e.file_name()).collect::<Vec<_>>();
    assert!(root.join("Projects").is_dir(), "renamed on Return; folder now has {entries:?}");
    assert!(!root.join("untitled folder").exists());
    assert_eq!(selected(&view, &cx).as_deref(), Some("Projects"));
}

#[gpui::test]
fn a_new_folder_clears_a_filter_that_hides_it(cx: &mut TestAppContext) {
    let root = fixture("newfolder-filter");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("/");
    cx.simulate_input("a");
    cx.simulate_keystrokes("enter");
    cx.simulate_keystrokes("cmd-shift-n");
    settle(&view, &mut cx);
    cx.run_until_parked();
    let pane = pane(&view, &cx);
    assert!(!pane.read_with(&cx, |pane, _| pane.is_filtering()), "the filter clears so the new row shows");
    assert_eq!(
        pane.read_with(&cx, |pane, cx| pane.rename_text(cx)).as_deref(),
        Some("untitled folder"),
        "the new folder's rename starts"
    );
    let handles = pane.read_with(&cx, |pane, cx| pane.text_focus_handles(cx));
    let focused = cx.update(|window, cx| window.focused(cx)).expect("the name field holds focus");
    assert_eq!(handles, vec![focused], "the visible name field owns focus");
    assert!(
        rows(&view, &cx).iter().any(|(name, _)| name == "untitled folder"),
        "the new row is visible: {:?}",
        rows(&view, &cx)
    );
}

#[gpui::test]
fn new_folder_arrives_and_its_rename_takes_typing_at_once(cx: &mut TestAppContext) {
    let root = fixture("newfolder-arrive");
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "full");
    cx.simulate_keystrokes("cmd-shift-n");
    settle(&view, &mut cx);
    cx.run_until_parked();
    let effects = pane(&view, &cx).read_with(&cx, |pane, cx| pane.effects(cx));
    assert!(
        effects.iter().any(|(name, fx)| name == "untitled folder" && *fx == Fx::Arrive),
        "the new folder arrives: {effects:?}"
    );
    cx.simulate_input("Projects");
    assert_eq!(
        pane(&view, &cx).read_with(&cx, |pane, cx| pane.rename_text(cx)).as_deref(),
        Some("Projects"),
        "typing goes into the field without waiting for the animation"
    );
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert!(root.join("Projects").is_dir());
}

#[gpui::test]
fn undo_arrives_in_full_motion(cx: &mut TestAppContext) {
    let root = fixture("undo-arrive");
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "full");
    cx.simulate_keystrokes("down down cmd-backspace");
    settle(&view, &mut cx);
    assert!(!root.join("notes.txt").exists());
    cx.simulate_keystrokes("cmd-z");
    settle(&view, &mut cx);
    assert!(root.join("notes.txt").exists(), "undo put it back");
    let effects = pane(&view, &cx).read_with(&cx, |pane, cx| pane.effects(cx));
    assert!(
        effects.iter().any(|(name, fx)| name == "notes.txt" && *fx == Fx::Arrive),
        "the restored row arrives: {effects:?}"
    );
}

#[gpui::test]
fn new_file_creates_and_renames(cx: &mut TestAppContext) {
    let root = fixture("newfile");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-alt-n");
    settle(&view, &mut cx);
    cx.run_until_parked();
    let renaming = pane(&view, &cx).read_with(&cx, |p, cx| p.rename_text(cx));
    assert_eq!(renaming.as_deref(), Some("untitled"), "the new file is created and its rename starts");
    assert!(root.join("untitled").is_file(), "an empty file, not a folder");
    cx.simulate_input("Notes");
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert!(root.join("Notes").is_file(), "renamed on Return");
    assert!(!root.join("untitled").exists());
    assert_eq!(selected(&view, &cx).as_deref(), Some("Notes"));

    // The palette lists it too.
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input(">new file");
    settle(&view, &mut cx);
    let rows = view.read_with(&cx, |w, _| w.palette_rows());
    assert!(rows.first().is_some_and(|row| row.contains("New File")), "{rows:?}");
}

#[gpui::test]
fn renaming_a_folder_with_a_dot_selects_the_whole_name(cx: &mut TestAppContext) {
    let root = fixture("rename-folder-dot");
    std::fs::create_dir_all(root.join("a.b")).expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    assert_eq!(selected(&view, &cx).as_deref(), Some("a.b"), "folders first: the cursor starts on a.b");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.simulate_input("c");
    let renaming = pane(&view, &cx).read_with(&cx, |p, cx| p.rename_text(cx));
    assert_eq!(renaming.as_deref(), Some("c"), "a folder's whole name is selected, dot included");
}

#[gpui::test]
fn a_late_change_batch_does_not_bring_back_a_deleted_file(cx: &mut TestAppContext) {
    let root = fixture("change-race");
    let (view, mut cx) = open(cx, &root);
    settle(&view, &mut cx);
    let folder = pane(&view, &cx).read_with(&cx, |p, _| p.test_folder());
    folder.update(&mut cx, |folder, _| folder.set_apply_delay(std::time::Duration::from_millis(300)));
    let executor = cx.executor();

    // Delete notes.txt and queue it; the poll picks the batch up but the apply
    // lands 300 ms later, so it is still in flight for the next batch.
    std::fs::remove_file(root.join("notes.txt")).expect("delete");
    folder.update(&mut cx, |folder, _| folder.queue_changes(vec![root.join("notes.txt")]));
    executor.advance_clock(std::time::Duration::from_millis(100));
    cx.run_until_parked();

    // A second batch whose stat predates the first apply used to carry the old
    // listing (with notes.txt) and put the deleted file back.
    std::fs::write(root.join("later.txt"), "x").expect("create");
    folder.update(&mut cx, |folder, _| folder.queue_changes(vec![root.join("later.txt")]));
    executor.advance_clock(std::time::Duration::from_millis(100));
    cx.run_until_parked();
    executor.advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();

    let names = folder.read_with(&cx, |folder, _| folder.names());
    assert!(!names.contains(&"notes.txt".to_owned()), "the deleted file stays gone: {names:?}");
    assert!(names.contains(&"later.txt".to_owned()), "the second batch still lands: {names:?}");
}

#[gpui::test]
fn preview_toggle_hides_it_in_list_and_columns(cx: &mut TestAppContext) {
    let root = fixture("preview-toggle");
    let (view, mut cx) = open(cx, &root);
    assert!(cx.debug_bounds("preview").is_some(), "list view shows the preview");
    cx.simulate_keystrokes("cmd-shift-p");
    settle(&view, &mut cx);
    assert!(cx.debug_bounds("preview").is_none(), "hiding removes the list preview");
    assert!(!cx.read(|cx| cx.global::<Ui>().settings.show_preview), "the setting is off");
    let text = std::fs::read_to_string(config_dir(&cx).join("settings.toml")).expect("settings file");
    assert!(text.contains("\nshow_preview = false\n"), "the choice is saved as a bool: {text}");
    assert_eq!(
        view.read_with(&cx, |w, cx| w.toggle_state(crate::palette::CommandId::TogglePreview, cx)),
        Some(false),
        "the palette chip follows"
    );

    cx.simulate_keystrokes("cmd-shift-p");
    settle(&view, &mut cx);
    assert!(cx.debug_bounds("preview").is_some(), "showing it again");

    cx.simulate_keystrokes("cmd-3");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("down down");
    settle(&view, &mut cx);
    assert!(cx.debug_bounds("preview-column").is_some(), "a file's preview is the last column");
    cx.simulate_keystrokes("cmd-shift-p");
    settle(&view, &mut cx);
    assert!(cx.debug_bounds("preview-column").is_none(), "hiding removes the preview column");
    assert_eq!(view.read_with(&cx, |w, cx| w.pane().read(cx).mode()), ViewMode::Columns);
}

#[gpui::test]
fn show_preview_false_in_settings_hides_it(cx: &mut TestAppContext) {
    let root = fixture("preview-setting");
    let (view, mut cx) = open(cx, &root);
    let text = std::fs::read_to_string(config_dir(&cx).join("settings.toml")).expect("settings file");
    assert!(text.contains("show_preview = true"), "the documented default: {text}");
    write_config(&view, &mut cx, "settings.toml", &text.replace("show_preview = true", "show_preview = false"));
    assert!(cx.debug_bounds("preview").is_none(), "live reload hides the preview");
    let focus = cx.update(|_, cx| cx.global::<Ui>().settings.show_preview);
    assert!(!focus);
}

#[gpui::test]
fn palette_find_opens_search(cx: &mut TestAppContext) {
    let root = fixture("palette-find");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    cx.simulate_input(">find");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    assert!(view.read_with(&cx, |w, _| w.palette.is_none()), "the palette closes");
    let focus =
        view.read_with(&cx, |w, cx| w.search.as_ref().expect("Find… opens search").read(cx).input_focus_handle(cx));
    assert!(cx.update(|window, _| focus.is_focused(window)), "focus is in the search field");
}

#[gpui::test]
fn palette_copy_path_copies_the_selection(cx: &mut TestAppContext) {
    let root = fixture("palette-copy-path");
    let (_view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down");
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    cx.simulate_input(">copy path");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let text = cx.read(|cx| crate::effects::recording(cx).and_then(|recorder| recorder.text()));
    assert_eq!(text, Some(root.join("beta").display().to_string()), "the palette ran Copy Path");
}

#[gpui::test]
fn plain_keys_stay_in_every_text_field(cx: &mut TestAppContext) {
    use crate::actions::{CopyFiles, CutFiles, MoveToTrash, PasteFiles, QuickLook, Rename, SelectAll, StartFilter};

    let root = fixture("plain-keys");
    let (view, mut cx) = open(cx, &root);
    // simulate_input skips AppKit's menu equivalents, so the real guard is that no list
    // action is on the focus path: a disabled menu item lets its plain key through.
    fn list_actions_off(field: &str, cx: &mut VisualTestContext) {
        for (action, name) in [
            (&StartFilter as &dyn gpui::Action, "Filter This Folder"),
            (&QuickLook, "Quick Look"),
            (&Rename, "Rename"),
            (&MoveToTrash, "Move to Trash"),
        ] {
            assert!(!action_available(action, cx), "{field}: {name} is off the path");
        }
    }

    // Rename: the list's actions must be off the focus path so AppKit passes plain keys through.
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(pane(&view, &cx).read_with(&cx, |p, _| p.is_renaming()), "Return starts rename");
    cx.simulate_input("a/b c");
    let (name, filtering) = pane(&view, &cx).read_with(&cx, |p, cx| (p.rename_text(cx), p.is_filtering()));
    assert_eq!(name.as_deref(), Some("a/b c"), "slash and space land in the name");
    assert!(!filtering, "slash did not start the filter");
    assert!(view.read_with(&cx, |w, _| w.search.is_none()), "space did not open Quick Look");
    list_actions_off("rename", &mut cx);
    assert!(action_available(&CopyFiles, &mut cx), "the field handles Copy");
    assert!(action_available(&CutFiles, &mut cx), "the field handles Cut");
    assert!(action_available(&PasteFiles, &mut cx), "the field handles Paste");
    assert!(action_available(&SelectAll, &mut cx), "the field handles Select All");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(!pane(&view, &cx).read_with(&cx, |p, _| p.is_renaming()), "Return ends the rename");
    settle(&view, &mut cx);

    // Filter field.
    cx.simulate_keystrokes("/");
    cx.run_until_parked();
    cx.simulate_input("a/b c");
    let text = pane(&view, &cx).read_with(&cx, |p, cx| p.filter_text(cx));
    assert_eq!(text.as_deref(), Some("a/b c"), "slash and space land in the filter");
    list_actions_off("filter", &mut cx);
    cx.simulate_keystrokes("escape");

    // Go to Folder (the palette on a path).
    cx.simulate_keystrokes("cmd-shift-g");
    cx.run_until_parked();
    cx.simulate_input("/ ");
    let text = view
        .read_with(&cx, |w, cx| w.palette.as_ref().map(|p| p.input.read(cx).text().to_owned()))
        .expect("Go to Folder stays open");
    assert!(text.ends_with("/ "), "slash and space land in Go to Folder, got {text:?}");
    list_actions_off("Go to Folder", &mut cx);
    cx.simulate_keystrokes("escape");

    // Search field.
    cx.simulate_keystrokes("cmd-f");
    cx.run_until_parked();
    cx.simulate_input("/ ");
    let text = view.read_with(&cx, |w, cx| w.search.as_ref().expect("search open").read(cx).query_text(cx));
    assert_eq!(text, "/ ", "slash and space land in the search field");
    assert!(!pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()), "search did not start the filter");
    list_actions_off("search", &mut cx);
    cx.simulate_keystrokes("escape");

    // Palette field.
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    cx.simulate_input("/ ");
    let text = view.read_with(&cx, |w, cx| w.palette.as_ref().expect("palette open").input.read(cx).text().to_owned());
    assert_eq!(text, "/ ", "slash and space land in the palette");
    list_actions_off("palette", &mut cx);
    cx.simulate_keystrokes("escape");
}

#[gpui::test]
fn cmd_z_in_a_text_field_undoes_typing_not_files(cx: &mut TestAppContext) {
    let root = fixture("field-undo");
    let (view, mut cx) = open(cx, &root);
    // A file operation to (wrongly) undo: trash notes.txt.
    cx.simulate_keystrokes("down down cmd-backspace");
    settle(&view, &mut cx);
    assert!(!root.join("notes.txt").exists());

    // Rename beta: the trash selected the previous row (beta), so it is already the cursor.
    cx.simulate_keystrokes("down down");
    assert_eq!(selected(&view, &cx).as_deref(), Some("beta"));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.simulate_input("gamma");
    cx.simulate_keystrokes("backspace backspace");
    let name = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, cx| p.rename_text(cx));
    assert_eq!(name(&cx).as_deref(), Some("gam"));

    cx.simulate_keystrokes("cmd-z");
    assert_eq!(name(&cx).as_deref(), Some("gamma"), "undo takes back the deletions as one step");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(name(&cx).as_deref(), Some("beta"), "undo takes back the typing as one step");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(name(&cx).as_deref(), Some("beta"), "nothing left to undo in the field");
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(name(&cx).as_deref(), Some("gamma"), "redo brings the typing back");
    settle(&view, &mut cx);
    assert!(!root.join("notes.txt").exists(), "the trash was never undone");
    cx.simulate_keystrokes("escape");

    // The same in the filter field, with the menu's Undo action.
    cx.simulate_keystrokes("/");
    cx.run_until_parked();
    cx.simulate_input("al");
    cx.dispatch_action(crate::actions::Undo);
    let text = pane(&view, &cx).read_with(&cx, |p, cx| p.filter_text(cx));
    assert_eq!(text.as_deref(), Some(""), "Edit ▸ Undo undoes the filter text");
    settle(&view, &mut cx);
    assert!(!root.join("notes.txt").exists(), "the trash was never undone");
    cx.simulate_keystrokes("escape");

    // Back in the list, ⌘Z undoes the file operation again.
    cx.simulate_keystrokes("cmd-z");
    settle(&view, &mut cx);
    assert!(root.join("notes.txt").exists(), "list ⌘Z undoes the trash");
}

/// Set `motion` in the test's settings.toml and reload, as a user would.
fn set_motion(cx: &mut VisualTestContext, mode: &str) {
    cx.update(|_, cx| {
        let dir = cx.global::<crate::ui::Ui>().config_dir.clone();
        std::fs::write(dir.join("settings.toml"), format!("motion = \"{mode}\"\n")).expect("write settings");
        crate::ui::reload_config(cx);
    });
}

fn trash_in_mode(mode: &str, cx: &mut TestAppContext) {
    let root = fixture(&format!("trash-{mode}"));
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, mode);
    cx.simulate_keystrokes("down down"); // notes.txt
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"));
    cx.simulate_keystrokes("cmd-backspace");
    settle(&view, &mut cx);
    assert!(!root.join("notes.txt").exists(), "{mode}: trashed");
    assert!(!names(&view, &cx).contains(&"notes.txt".to_owned()), "{mode}: gone from the list");
    assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "{mode}: the previous row is selected, as in Finder");
    assert!(pane(&view, &cx).read_with(&cx, |pane, cx| pane.error(cx).is_none()), "{mode}: no error");
}

#[gpui::test]
fn trash_in_off_motion_leaves_the_listing(cx: &mut TestAppContext) {
    trash_in_mode("off", cx);
}

#[gpui::test]
fn trash_in_low_motion_leaves_the_listing(cx: &mut TestAppContext) {
    trash_in_mode("low", cx);
}

#[gpui::test]
fn trash_in_full_motion_leaves_the_listing(cx: &mut TestAppContext) {
    trash_in_mode("full", cx);
}

#[gpui::test]
fn trash_in_full_motion_in_columns_leaves_the_listing(cx: &mut TestAppContext) {
    let root = fixture("trash-full-columns");
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "full");
    cx.simulate_keystrokes("cmd-3");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("down down cmd-backspace");
    settle(&view, &mut cx);
    assert!(!root.join("notes.txt").exists(), "trashed from the column");
    assert!(!names(&view, &cx).contains(&"notes.txt".to_owned()), "gone from the column");
    assert!(pane(&view, &cx).read_with(&cx, |pane, cx| pane.error(cx).is_none()), "no error");
    assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "the previous row is selected, as in Finder");
    cx.simulate_keystrokes("up");
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "↑ moves from the selected row");
}

#[gpui::test]
fn trashing_a_middle_row_selects_the_next_row(cx: &mut TestAppContext) {
    let root = fixture("trash-middle-next");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down"); // beta
    assert_eq!(selected(&view, &cx).as_deref(), Some("beta"));
    cx.simulate_keystrokes("cmd-backspace");
    settle(&view, &mut cx);
    assert!(!root.join("beta").exists(), "trashed");
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "the next row");
    assert_eq!(selected_names(&view, &cx), ["notes.txt"], "only the next row");
}

#[gpui::test]
fn trashing_a_multi_selection_selects_the_row_after_it(cx: &mut TestAppContext) {
    let root = fixture("trash-multi-next");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("shift-down"); // alpha and beta, cursor beta
    assert_eq!(selected_names(&view, &cx), ["alpha", "beta"]);
    cx.simulate_keystrokes("cmd-backspace");
    settle(&view, &mut cx);
    assert!(!root.join("alpha").exists() && !root.join("beta").exists(), "both trashed");
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "the row after the selection");
    assert_eq!(selected_names(&view, &cx), ["notes.txt"], "only the next row");
}

#[gpui::test]
fn trash_three_items_in_full_motion(cx: &mut TestAppContext) {
    let root = fixture("trash-three");
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "full");
    cx.simulate_keystrokes("cmd-a cmd-backspace");
    settle(&view, &mut cx);
    assert!(names(&view, &cx).is_empty(), "all three rows left the list");
    assert!(!root.join("alpha").exists() && !root.join("beta").exists() && !root.join("notes.txt").exists());
}

#[gpui::test]
fn trash_five_hundred_items_in_full_motion(cx: &mut TestAppContext) {
    let root = std::env::temp_dir().join(format!("crow-app-trash-500-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create fixture");
    for i in 0..500 {
        std::fs::write(root.join(format!("file-{i:03}.txt")), "x").expect("create fixture");
    }
    let root = root.canonicalize().expect("canonical fixture path");
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "full");
    cx.simulate_keystrokes("cmd-a cmd-backspace");
    settle(&view, &mut cx);
    assert!(names(&view, &cx).is_empty(), "all 500 rows left the list");
    assert!(!root.join("file-000.txt").exists() && !root.join("file-499.txt").exists());
    assert!(pane(&view, &cx).read_with(&cx, |pane, cx| pane.error(cx).is_none()), "no error");
}

#[gpui::test]
fn trash_and_undo(cx: &mut TestAppContext) {
    let root = fixture("trash");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down down"); // notes.txt
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"));
    cx.simulate_keystrokes("cmd-backspace");
    settle(&view, &mut cx);
    assert!(!root.join("notes.txt").exists());
    assert!(!names(&view, &cx).contains(&"notes.txt".to_owned()));
    cx.simulate_keystrokes("cmd-z");
    settle(&view, &mut cx);
    assert_eq!(std::fs::read_to_string(root.join("notes.txt")).ok().as_deref(), Some("hello\n"), "undo puts it back");
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "…and selects it");
}

#[gpui::test]
fn duplicate_and_paste_into_another_folder(cx: &mut TestAppContext) {
    let root = fixture("dup");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down down cmd-d");
    settle(&view, &mut cx);
    assert!(root.join("notes copy.txt").exists());
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes copy.txt"), "the duplicate is selected");

    cx.simulate_keystrokes("cmd-c up up up cmd-down"); // copy, then into alpha
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"));
    cx.simulate_keystrokes("cmd-v");
    settle(&view, &mut cx);
    assert!(root.join("alpha/notes copy.txt").exists(), "pasted into the folder you are in");
    assert!(root.join("notes copy.txt").exists(), "copy leaves the original");
}

/// H4 #25: a 100k-file copy is ~10 MB of file URLs. Pasting it into the palette
/// used to reshape the whole field and score the folder with the whole query.
#[gpui::test]
fn a_huge_paste_into_the_palette_stays_quick(cx: &mut TestAppContext) {
    let root = fixture("huge-paste");
    let (_view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-shift-g");
    let text = "a".repeat(10_000_000);
    cx.update(|_, cx| crate::effects::write_text(text, cx));
    let started = Instant::now();
    cx.simulate_keystrokes("cmd-v");
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_millis(250), "a 10 MB paste took {elapsed:?}");
}

/// H4 #25: a huge path query must not shape a multi-MB missing-path label.
#[gpui::test]
fn a_huge_path_query_stays_quick(cx: &mut TestAppContext) {
    let root = fixture("huge-path");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-shift-g");
    let huge = format!("/nope-{}/x", "a".repeat(10_000_000));
    let started = Instant::now();
    view.update(&mut cx, |view, cx| view.refresh_palette(&huge, cx));
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_millis(500), "a 10 MB path query took {elapsed:?}");
}

#[gpui::test]
fn conflicts_ask_first_and_keep_both_by_default(cx: &mut TestAppContext) {
    let root = fixture("conflict");
    std::fs::write(root.join("alpha/notes.txt"), "already here").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down down cmd-c up up cmd-down cmd-v");
    settle(&view, &mut cx);
    assert!(sheet_open(&view, &cx), "a conflict opens a sheet instead of overwriting");
    assert_eq!(std::fs::read_to_string(root.join("alpha/notes.txt")).ok().as_deref(), Some("already here"));
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert!(!sheet_open(&view, &cx));
    assert_eq!(std::fs::read_to_string(root.join("alpha/notes 2.txt")).ok().as_deref(), Some("hello\n"));
    assert_eq!(std::fs::read_to_string(root.join("alpha/notes.txt")).ok().as_deref(), Some("already here"));
}

#[gpui::test]
fn permanent_delete_needs_confirmation(cx: &mut TestAppContext) {
    let root = fixture("delete");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down down cmd-alt-backspace");
    settle(&view, &mut cx);
    assert!(sheet_open(&view, &cx));
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    assert!(root.join("notes.txt").exists(), "Escape cancels");
    cx.simulate_keystrokes("cmd-alt-backspace enter");
    settle(&view, &mut cx);
    assert!(!root.join("notes.txt").exists(), "Return confirms");
}

/// Drag `paths` in from outside the window and drop them on the element `selector`.
///
/// The test platform makes a newly drawn frame the one that is hit-tested only at
/// the next input event (the app presents frames continuously). So the pointer
/// first moves over the target, as it does in a real drag, before the drop events.
fn drop_on(cx: &mut VisualTestContext, selector: &'static str, paths: Vec<PathBuf>) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let at = cx.debug_bounds(selector).unwrap_or_else(|| panic!("no element {selector}")).center();
    drop_at(cx, at, paths);
}

/// Drop `paths` from another app at `at`.
fn drop_at(cx: &mut VisualTestContext, at: gpui::Point<gpui::Pixels>, paths: Vec<PathBuf>) {
    cx.simulate_mouse_move(at, None, gpui::Modifiers::default());
    cx.run_until_parked();
    cx.simulate_event(gpui::FileDropEvent::Entered { position: at, paths: gpui::ExternalPaths(paths.into()) });
    cx.simulate_event(gpui::FileDropEvent::Pending { position: at });
    cx.simulate_event(gpui::FileDropEvent::Submit { position: at });
}

#[gpui::test]
fn platform_drag_updates_keep_one_list_target(cx: &mut TestAppContext) {
    let root = fixture("drop-steady-list");
    let (view, mut cx) = open(cx, &root);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let alpha = cx.debug_bounds("row-alpha").expect("alpha row").center();
    let notes = cx.debug_bounds("row-notes.txt").expect("notes row").center();
    let pane = view.read_with(&cx, |window, _| window.tabs.active().clone());
    let target = |cx: &VisualTestContext| pane.read_with(cx, |pane, _| pane.drop_row());

    cx.simulate_event(gpui::FileDropEvent::Entered {
        position: alpha,
        paths: gpui::ExternalPaths(vec![root.join("dropped.txt")].into()),
    });
    cx.simulate_event(gpui::FileDropEvent::Pending { position: alpha });
    cx.run_until_parked();
    assert_eq!(target(&cx), Some(0), "the row under the pointer is the target");
    for _ in 0..8 {
        cx.simulate_event(gpui::FileDropEvent::Pending { position: alpha });
        cx.run_until_parked();
        assert_eq!(target(&cx), Some(0), "platform updates keep one steady target");
    }
    cx.simulate_event(gpui::FileDropEvent::Pending { position: notes });
    cx.run_until_parked();
    assert_eq!(target(&cx), Some(2), "moving the pointer moves the target");
    cx.simulate_event(gpui::FileDropEvent::Exited);
    cx.run_until_parked();
    assert_eq!(target(&cx), None, "leaving the window clears it");
}

#[gpui::test]
fn platform_drag_updates_keep_one_column_target(cx: &mut TestAppContext) {
    let root = fixture("drop-steady-columns");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3");
    settle(&view, &mut cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let pane = view.read_with(&cx, |window, _| window.tabs.active().clone());
    assert_eq!(pane.read_with(&cx, |pane, _| pane.mode()), ViewMode::Columns, "column view is active");
    let at = gpui::point(px(320.), px(90.));
    cx.simulate_event(gpui::FileDropEvent::Entered {
        position: at,
        paths: gpui::ExternalPaths(vec![root.join("dropped.txt")].into()),
    });
    cx.simulate_event(gpui::FileDropEvent::Pending { position: at });
    cx.run_until_parked();
    assert_eq!(pane.read_with(&cx, |pane, _| pane.drop_column()), Some((0, 1)));
    for _ in 0..8 {
        cx.simulate_event(gpui::FileDropEvent::Pending { position: at });
        cx.run_until_parked();
        assert_eq!(
            pane.read_with(&cx, |pane, _| pane.drop_column()),
            Some((0, 1)),
            "platform updates keep one steady target in a column"
        );
    }
    cx.simulate_event(gpui::FileDropEvent::Exited);
    cx.run_until_parked();
    assert_eq!(pane.read_with(&cx, |pane, _| pane.drop_column()), None);
}

#[gpui::test]
fn sidebar_drop_highlight_is_steady_while_the_pointer_stays_on_a_row(cx: &mut TestAppContext) {
    let root = fixture("drop-steady");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("alpha"), &root.join("beta")]));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let first = cx.debug_bounds("sidebar-0").expect("first favorite row").center();
    let second = cx.debug_bounds("sidebar-1").expect("second favorite row").center();
    let header = cx.debug_bounds("sidebar-header-Favorites").expect("favorites title").center();
    let sidebar = view.read_with(&cx, |w, _| w.sidebar.clone());
    let target = |cx: &VisualTestContext| sidebar.read_with(cx, |s, _| s.drop_target());

    let paths = gpui::ExternalPaths(vec![root.join("notes.txt")].into());
    cx.simulate_event(gpui::FileDropEvent::Entered { position: second, paths });
    cx.simulate_event(gpui::FileDropEvent::Pending { position: second });
    cx.run_until_parked();
    assert_eq!(target(&cx), Some((true, 1)), "the row under the pointer is highlighted");
    assert!(sidebar_looks(&view, &mut cx)[1].drop, "the row's look carries the drop mark");
    let outlined = |cx: &mut VisualTestContext, selector: &'static str| {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let row = cx.debug_bounds(selector).expect("target");
        cx.debug_bounds("drop-outline").is_some_and(|outline| outline == row)
    };
    assert!(outlined(&mut cx, "sidebar-1"), "the target row is outlined, like list and column targets");
    for _ in 0..8 {
        cx.simulate_event(gpui::FileDropEvent::Pending { position: second });
        cx.run_until_parked();
        assert_eq!(target(&cx), Some((true, 1)), "…and stays highlighted while the pointer rests on it");
    }
    cx.simulate_event(gpui::FileDropEvent::Pending { position: first });
    cx.run_until_parked();
    assert_eq!(target(&cx), Some((true, 0)), "moving to another row moves the highlight");
    cx.simulate_event(gpui::FileDropEvent::Pending { position: header });
    cx.run_until_parked();
    assert_eq!(target(&cx), Some((false, 0)), "a group title highlights as a drop target");
    assert!(outlined(&mut cx, "sidebar-header-Favorites"), "…with the same outline");
    cx.simulate_event(gpui::FileDropEvent::Exited);
    cx.run_until_parked();
    assert_eq!(target(&cx), None, "leaving the window clears it");
    assert!(sidebar_looks(&view, &mut cx).iter().all(|look| !look.drop), "…and every row's drop mark goes");
}

#[gpui::test]
fn dragging_a_row_onto_a_sidebar_favorite_moves_it(cx: &mut TestAppContext) {
    let root = fixture("drag-sidebar");
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "sidebar.toml", &favorites_toml(&[&root.join("beta")]));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let from = cx.debug_bounds("row-notes.txt").expect("notes.txt row").center();
    let to = cx.debug_bounds("sidebar-0").expect("beta favorite row").center();
    cx.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(to, Some(gpui::MouseButton::Left), gpui::Modifiers::default());
    cx.simulate_mouse_up(to, gpui::MouseButton::Left, gpui::Modifiers::default());
    settle(&view, &mut cx);
    assert!(root.join("beta/notes.txt").exists(), "the dragged file moved into the favorite's folder");
}

#[gpui::test]
fn deactivating_the_window_promotes_an_active_drag(cx: &mut TestAppContext) {
    struct Probe {
        fired: std::rc::Rc<std::cell::Cell<bool>>,
        _activation: Subscription,
    }
    impl Render for Probe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let fired = self.fired.clone();
            div()
                .id("probe")
                .debug_selector(|| "probe".into())
                .size(px(80.))
                .on_drag((), |_, _, _, cx| cx.new(|_| gpui::Empty))
                .external_drag_payload::<()>(move |_, _, _| {
                    fired.set(true);
                    Some(gpui::ExternalDragPayload::Files(gpui::FileDragPaths::default()))
                })
        }
    }

    let fired = std::rc::Rc::new(std::cell::Cell::new(false));
    let fired_for_view = fired.clone();
    let (_probe, cx) = cx.add_window_view(|window, cx| {
        let activation = cx.observe_window_activation(window, |_, window, cx| {
            if !window.is_window_active() {
                window.promote_active_drag_to_platform(cx);
            }
        });
        Probe { fired: fired_for_view, _activation: activation }
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let center = cx.debug_bounds("probe").expect("probe").center();
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(
        center + gpui::point(px(20.), px(20.)),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    assert!(cx.update(|_, cx| cx.has_active_drag()), "the drag started in-window");
    cx.deactivate_window();
    assert!(fired.get(), "deactivation hands the drag to the platform");
}

#[gpui::test]
fn escape_cancels_a_drag_and_returns_the_combo(cx: &mut TestAppContext) {
    let root = fixture("drag-escape");
    let (_view, mut cx) = open(cx, &root);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let from = cx.debug_bounds("row-notes.txt").expect("notes.txt row").center();
    cx.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(
        from + gpui::point(px(120.), px(40.)),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    assert!(cx.update(|_, cx| cx.has_active_drag()), "the drag started in-window");

    cx.simulate_keystrokes("escape");
    assert!(!cx.update(|_, cx| cx.has_active_drag()), "Escape ends the drag");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("drag-combo").is_some(), "the combo is still flying back");

    // The frame that sees the flight finished must not draw the combo: nothing asks
    // for another frame after it, so whatever it paints stays on screen.
    cx.executor().advance_clock(std::time::Duration::from_millis(300));
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("drag-combo").is_none(), "the combo landed and disappeared");
}

#[gpui::test]
fn dragging_a_row_onto_a_folder_moves_it(cx: &mut TestAppContext) {
    let root = fixture("drag-row");
    let (view, mut cx) = open(cx, &root);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let from = cx.debug_bounds("row-notes.txt").expect("notes.txt row").center();
    let to = cx.debug_bounds("row-alpha").expect("alpha row").center();
    cx.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(to, Some(gpui::MouseButton::Left), gpui::Modifiers::default());
    cx.simulate_mouse_up(to, gpui::MouseButton::Left, gpui::Modifiers::default());
    settle(&view, &mut cx);
    assert!(root.join("alpha/notes.txt").exists(), "the dragged file moved into the folder under the pointer");
    assert!(!root.join("notes.txt").exists(), "…and left its old place");
}

#[gpui::test]
fn dropping_a_file_back_into_its_expanded_folder_is_a_no_op(cx: &mut TestAppContext) {
    let root = fixture("drop-self");
    std::fs::remove_dir_all(root.join("alpha/inner")).expect("drop fixture dir");
    std::fs::write(root.join("alpha/inner.txt"), "hello\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "off");
    cx.simulate_keystrokes("right");
    settle(&view, &mut cx);

    drop_on(&mut cx, "row-inner.txt", vec![root.join("alpha/inner.txt")]);
    settle(&view, &mut cx);
    assert!(root.join("alpha/inner.txt").exists(), "the file stays in its folder");
    assert!(!root.join("inner.txt").exists(), "and does not move to the root");
}

#[gpui::test]
fn dropping_from_an_expanded_folder_into_a_sibling_moves_it(cx: &mut TestAppContext) {
    let root = fixture("drop-sibling");
    std::fs::remove_dir_all(root.join("alpha/inner")).expect("drop fixture dir");
    std::fs::write(root.join("alpha/inner.txt"), "hello\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "off");
    cx.simulate_keystrokes("right");
    settle(&view, &mut cx);

    drop_on(&mut cx, "row-beta", vec![root.join("alpha/inner.txt")]);
    settle(&view, &mut cx);
    assert!(root.join("beta/inner.txt").exists(), "the file moved into the sibling");
    assert!(!root.join("alpha/inner.txt").exists(), "and left the expanded folder");
}

#[gpui::test]
fn dropping_onto_a_file_inside_an_expanded_folder_targets_its_folder(cx: &mut TestAppContext) {
    let root = fixture("drop-nested");
    std::fs::remove_dir_all(root.join("alpha/inner")).expect("drop fixture dir");
    std::fs::write(root.join("alpha/inner.txt"), "hello\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "off");
    cx.simulate_keystrokes("right");
    settle(&view, &mut cx);

    drop_on(&mut cx, "row-inner.txt", vec![root.join("notes.txt")]);
    settle(&view, &mut cx);
    assert!(root.join("alpha/notes.txt").exists(), "the file moved into the expanded folder");
    assert!(!root.join("notes.txt").exists(), "and left the root");
}

#[gpui::test]
fn nested_expanded_folders_target_the_innermost_folder(cx: &mut TestAppContext) {
    let root = fixture("drop-nested-two");
    std::fs::write(root.join("alpha/inner/deep.txt"), "hello\n").expect("create fixture");
    std::fs::write(root.join("alpha/zz.txt"), "hello\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "off");
    cx.simulate_keystrokes("right");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("down right");
    settle(&view, &mut cx);
    let pane = pane(&view, &cx);
    let rows = pane.read_with(&cx, |pane, cx| pane.rows(cx));
    let at = |name: &str| rows.iter().position(|(row, _)| row == name).expect("row");
    assert_eq!(
        rows.iter().map(|(name, depth)| (name.as_str(), *depth)).collect::<Vec<_>>()[at("alpha")..=at("zz.txt")],
        [("alpha", 0), ("inner", 1), ("deep.txt", 2), ("zz.txt", 1)],
        "both folders are expanded"
    );
    let hover = |cx: &mut VisualTestContext, selector: &'static str| {
        let position = cx.debug_bounds(selector).expect("row").center();
        cx.simulate_event(gpui::FileDropEvent::Entered {
            position,
            paths: gpui::ExternalPaths(vec![root.join("notes.txt")].into()),
        });
        cx.simulate_event(gpui::FileDropEvent::Pending { position });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let target = pane.read_with(cx, |pane, _| pane.drop_target().cloned());
        cx.simulate_event(gpui::FileDropEvent::Exited);
        target
    };

    assert_eq!(
        hover(&mut cx, "row-deep.txt"),
        Some(crate::pane::DropTarget {
            folder: root.join("alpha/inner"),
            column: 0,
            rows: Some(at("inner")..at("zz.txt"))
        }),
        "a file two levels down targets its own folder, not the outer one"
    );
    assert_eq!(
        hover(&mut cx, "row-zz.txt"),
        Some(crate::pane::DropTarget {
            folder: root.join("alpha"),
            column: 0,
            rows: Some(at("alpha")..at("zz.txt") + 1)
        }),
        "a file after a nested block targets the outer folder, which spans the nested block"
    );
}

/// Hover `paths` from another app over the element `selector` (plus `nudge`), draw,
/// and return the pane's drop target and whether an outline was drawn.
fn hover_target(
    cx: &mut VisualTestContext,
    pane: &Entity<crate::pane::Pane>,
    selector: &'static str,
    nudge: gpui::Point<gpui::Pixels>,
    paths: Vec<PathBuf>,
) -> (Option<crate::pane::DropTarget>, bool) {
    let position = cx.debug_bounds(selector).expect("hover element").center() + nudge;
    cx.simulate_event(gpui::FileDropEvent::Entered { position, paths: gpui::ExternalPaths(paths.into()) });
    cx.simulate_event(gpui::FileDropEvent::Pending { position });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let target = pane.read_with(cx, |pane, _| pane.drop_target().cloned());
    let outlined = cx.debug_bounds("drop-outline").is_some();
    cx.simulate_event(gpui::FileDropEvent::Exited);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    (target, outlined)
}

#[gpui::test]
fn a_drop_into_the_shown_folder_outlines_the_list(cx: &mut TestAppContext) {
    let root = fixture("drop-root-list");
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "off");
    cx.simulate_keystrokes("right");
    settle(&view, &mut cx);
    let pane = pane(&view, &cx);
    let none = gpui::point(px(0.), px(0.));
    let below = gpui::point(px(0.), px(120.));
    let shown = Some(crate::pane::DropTarget { folder: root.clone(), column: 0, rows: None });

    assert_eq!(
        hover_target(&mut cx, &pane, "row-notes.txt", none, vec![root.join("alpha/inner")]),
        (shown.clone(), true),
        "a nested item over a top-level file goes to the shown folder, outlined as a whole"
    );
    assert_eq!(
        hover_target(&mut cx, &pane, "row-notes.txt", below, vec![root.join("alpha/inner")]),
        (shown, true),
        "so does the empty space below the rows"
    );
    assert_eq!(
        hover_target(&mut cx, &pane, "row-notes.txt", below, vec![root.join("notes.txt")]),
        (None, false),
        "an item already in the shown folder marks nothing"
    );
    assert_eq!(
        hover_target(&mut cx, &pane, "row-inner", none, vec![root.join("alpha/inner")]),
        (None, false),
        "nor does a folder over itself"
    );
    assert_eq!(
        hover_target(&mut cx, &pane, "row-beta", none, vec![root.join("alpha/inner")]),
        (Some(crate::pane::DropTarget { folder: root.join("beta"), column: 0, rows: Some(2..3) }), true),
        "a collapsed folder is outlined like any other target"
    );
}

#[gpui::test]
fn column_view_outlines_the_column_or_the_folder_row(cx: &mut TestAppContext) {
    let root = fixture("drop-root-columns");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3");
    settle(&view, &mut cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let pane = pane(&view, &cx);
    let none = gpui::point(px(0.), px(0.));

    assert_eq!(
        hover_target(&mut cx, &pane, "column-0-row-notes.txt", none, vec![root.join("alpha/inner")]),
        (Some(crate::pane::DropTarget { folder: root.clone(), column: 0, rows: None }), true),
        "a file row in a column drops into that column's folder"
    );
    assert_eq!(
        hover_target(&mut cx, &pane, "column-0-row-beta", none, vec![root.join("alpha/inner")]),
        (Some(crate::pane::DropTarget { folder: root.join("beta"), column: 0, rows: Some(1..2) }), true),
        "a folder row is the target itself"
    );
    assert_eq!(
        hover_target(&mut cx, &pane, "column-0-row-notes.txt", none, vec![root.join("notes.txt")]),
        (None, false),
        "an item already in the column's folder marks nothing"
    );
}

#[gpui::test]
fn expanded_folder_drop_target_is_outlined(cx: &mut TestAppContext) {
    let root = fixture("drop-outline");
    std::fs::remove_dir_all(root.join("alpha/inner")).expect("drop fixture dir");
    std::fs::write(root.join("alpha/inner.txt"), "hello\n").expect("create fixture");
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "off");
    cx.simulate_keystrokes("right");
    settle(&view, &mut cx);
    let pane = pane(&view, &cx);
    let alpha_row =
        pane.read_with(&cx, |pane, cx| pane.rows(cx).iter().position(|(name, _)| name == "alpha").expect("alpha row"));
    let at = cx.debug_bounds("row-inner.txt").expect("inner row").center();
    cx.simulate_event(gpui::FileDropEvent::Entered {
        position: at,
        paths: gpui::ExternalPaths(vec![root.join("notes.txt")].into()),
    });
    cx.simulate_event(gpui::FileDropEvent::Pending { position: at });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert_eq!(
        pane.read_with(&cx, |pane, _| pane.drop_target().cloned()),
        Some(crate::pane::DropTarget { folder: root.join("alpha"), column: 0, rows: Some(alpha_row..alpha_row + 2) }),
        "a file row inside the expanded folder targets that folder"
    );
    assert!(cx.debug_bounds("drop-outline").is_some(), "the whole folder block is outlined");

    cx.simulate_event(gpui::FileDropEvent::Exited);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert_eq!(pane.read_with(&cx, |pane, _| pane.drop_target().cloned()), None);
    assert!(cx.debug_bounds("drop-outline").is_none(), "leaving clears the outline");
}

#[gpui::test]
fn dropping_on_a_folder_row_in_full_motion_moves_and_leaves_the_listing(cx: &mut TestAppContext) {
    let root = fixture("drop-row-full");
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "full");
    drop_on(&mut cx, "row-alpha", vec![root.join("notes.txt")]);
    settle(&view, &mut cx);
    assert!(root.join("alpha/notes.txt").exists(), "dropped onto alpha: moved into it");
    assert!(!root.join("notes.txt").exists(), "…and left its old place");
    assert!(!names(&view, &cx).contains(&"notes.txt".to_owned()), "the source row left the list");
}

#[gpui::test]
fn a_move_starts_effects_on_the_source_and_the_target(cx: &mut TestAppContext) {
    let root = fixture("move-effects");
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "full");
    let pane = pane(&view, &cx);
    cx.update(|_, cx| {
        pane.update(cx, |pane, cx| {
            pane.start_effect(&[root.join("notes.txt")], Fx::MoveOut, cx);
            pane.start_effect(&[root.join("alpha")], Fx::Receive, cx);
        });
    });
    let effects = pane.read_with(&cx, |pane, cx| pane.effects(cx));
    assert!(effects.iter().any(|(name, fx)| name == "notes.txt" && *fx == Fx::MoveOut), "{effects:?}");
    assert!(effects.iter().any(|(name, fx)| name == "alpha" && *fx == Fx::Receive), "{effects:?}");
}

#[gpui::test]
fn dropping_on_a_folder_row_moves_into_that_folder(cx: &mut TestAppContext) {
    let root = fixture("drop-row");
    let (view, mut cx) = open(cx, &root);
    drop_on(&mut cx, "row-alpha", vec![root.join("notes.txt")]);
    settle(&view, &mut cx);
    assert!(root.join("alpha/notes.txt").exists(), "dropped onto alpha: moved into it");
    assert!(!root.join("notes.txt").exists(), "same volume: a move, like Finder");

    drop_on(&mut cx, "row-alpha", vec![root.join("alpha")]);
    settle(&view, &mut cx);
    assert!(root.join("alpha").is_dir(), "a folder dropped onto itself stays put");
    let message = cx.update(|_, cx| cx.global::<Operations>().message.clone());
    assert!(message.is_none_or(|(_, error)| !error), "…without an error");
}

#[gpui::test]
fn moving_the_selected_item_away_in_columns_selects_the_previous_row(cx: &mut TestAppContext) {
    let root = fixture("move-columns-selection");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("down down"); // notes.txt
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"));
    drop_on(&mut cx, "column-0-row-alpha", vec![root.join("notes.txt")]);
    settle(&view, &mut cx);
    assert!(root.join("alpha/notes.txt").exists(), "moved into alpha");
    assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "the previous row is selected, as in Finder");
}

#[gpui::test]
fn dropping_on_the_list_moves_into_the_open_folder(cx: &mut TestAppContext) {
    let root = fixture("drop-list");
    let elsewhere = fixture("drop-elsewhere");
    let (view, mut cx) = open(cx, &root);
    drop_on(&mut cx, "row-notes.txt", vec![elsewhere.join("notes.txt")]);
    settle(&view, &mut cx);
    assert!(sheet_open(&view, &cx), "a file row is not a target: the drop lands in the folder, where the name clashes");
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert!(root.join("notes 2.txt").exists(), "kept both");
    assert!(!elsewhere.join("notes.txt").exists(), "moved, not copied");
}

#[gpui::test]
fn dropping_in_column_view_targets_the_folder_under_the_pointer(cx: &mut TestAppContext) {
    let root = fixture("drop-columns");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3");
    settle(&view, &mut cx);
    drop_on(&mut cx, "column-0-row-beta", vec![root.join("notes.txt")]);
    settle(&view, &mut cx);
    assert!(root.join("beta/notes.txt").exists());
}

// ----- U9: contextual tips -----

/// `root/` with `n` files, for the scroll trigger.
fn long_fixture(name: &str, n: usize) -> PathBuf {
    let root = std::env::temp_dir().join(format!("crow-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create fixture");
    for i in 0..n {
        std::fs::write(root.join(format!("f{i:03}.txt")), "").expect("create fixture");
    }
    root.canonicalize().expect("canonical fixture path")
}

fn tip(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> Option<TipId> {
    view.read_with(cx, |w, _| w.tip)
}

fn tips_json(cx: &VisualTestContext) -> String {
    let file = cx.read(|cx| cx.global::<Ui>().state_dir.join(crow_core::TIPS_FILE));
    std::fs::read_to_string(file).unwrap_or_default()
}

/// A pointer scroll of `lines` up (negative delta) over `selector`.
fn scroll(cx: &mut VisualTestContext, selector: &'static str, lines: f32) {
    let bounds = cx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} is laid out"));
    cx.simulate_event(gpui::ScrollEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Lines(gpui::point(0., -lines)),
        ..Default::default()
    });
}

/// Call `offer_tip` as a trigger would, with the test window.
fn offer(view: &Entity<CrowWindow>, id: TipId, cx: &mut VisualTestContext) {
    let window = cx.update(|_, cx| cx.windows()[0]);
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| view.offer_tip(id, window, cx));
    })
    .expect("offer the tip");
}

#[gpui::test]
fn tip_got_it_retires_it(cx: &mut TestAppContext) {
    let root = long_fixture("tip-got-it", 250);
    let (view, mut cx) = open(cx, &root);
    scroll(&mut cx, "file-list", 60.);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), Some(TipId::Filter), "a long folder scroll suggests /");

    let button = cx.debug_bounds("tip-got-it").expect("the tip's button is laid out");
    cx.simulate_click(button.center(), Default::default());
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), None, "Got it closes the tip");
    assert!(tips_json(&cx).contains("\"filter\""), "retired in the state file: {}", tips_json(&cx));

    scroll(&mut cx, "file-list", 60.);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), None, "shown once, never again");
}

#[gpui::test]
fn tip_closes_when_the_folder_changes(cx: &mut TestAppContext) {
    let root = long_fixture("tip-folder-change", 250);
    let (view, mut cx) = open(cx, &root);
    scroll(&mut cx, "file-list", 60.);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), Some(TipId::Filter));

    cx.simulate_keystrokes("cmd-up");
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), None, "leaving the folder closes it");

    cx.simulate_keystrokes("cmd-[");
    settle(&view, &mut cx);
    scroll(&mut cx, "file-list", 60.);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), None, "and it was already spent");
}

#[gpui::test]
fn tip_hints_off_suppresses_and_hides(cx: &mut TestAppContext) {
    let root = long_fixture("tip-hints-off", 250);
    let (view, mut cx) = open(cx, &root);
    write_config(&view, &mut cx, "settings.toml", "hints = \"off\"\n");
    scroll(&mut cx, "file-list", 60.);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), None, "hints = off shows no tips");

    write_config(&view, &mut cx, "settings.toml", "hints = \"auto\"\n");
    scroll(&mut cx, "file-list", 60.);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), Some(TipId::Filter), "auto allows tips");

    write_config(&view, &mut cx, "settings.toml", "hints = \"off\"\n");
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), None, "turning hints off hides a tip on screen");
}

#[gpui::test]
fn tip_never_shows_while_busy(cx: &mut TestAppContext) {
    let root = long_fixture("tip-busy", 250);
    let (view, mut cx) = open(cx, &root);

    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert!(pane(&view, &cx).read_with(&cx, |pane, _| pane.is_renaming()), "a rename is open");
    offer(&view, TipId::Filter, &mut cx);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), None, "no tip during a rename");
    assert!(!tips_json(&cx).contains("\"filter\""), "and it is not retired");
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);

    for (open_keys, close_keys, open) in
        [("cmd-f", "escape", "search"), ("cmd-k", "escape", "palette"), ("cmd-alt-l", "cmd-alt-l", "activity")]
    {
        cx.simulate_keystrokes(open_keys);
        settle(&view, &mut cx);
        offer(&view, TipId::Filter, &mut cx);
        settle(&view, &mut cx);
        assert_eq!(tip(&view, &cx), None, "no tip while {open} is open");
        assert!(!tips_json(&cx).contains("\"filter\""), "{open} does not retire it");
        cx.simulate_keystrokes(close_keys);
        settle(&view, &mut cx);
    }

    scroll(&mut cx, "file-list", 60.);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), Some(TipId::Filter), "the trigger still works after the overlays");

    cx.simulate_keystrokes("cmd-f");
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), None, "opening search closes a shown tip");
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
}

#[gpui::test]
fn tip_only_one_on_screen(cx: &mut TestAppContext) {
    let root = long_fixture("tip-one", 250);
    let (view, mut cx) = open(cx, &root);
    scroll(&mut cx, "file-list", 60.);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), Some(TipId::Filter));

    offer(&view, TipId::Search, &mut cx);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), Some(TipId::Filter), "the first tip stays; the second is refused");
}

#[gpui::test]
fn tip_scrolling_a_long_folder_suggests_the_filter_once(cx: &mut TestAppContext) {
    let root = long_fixture("tip-scroll", 250);
    let (view, mut cx) = open(cx, &root);
    scroll(&mut cx, "file-list", 60.);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), Some(TipId::Filter));

    let button = cx.debug_bounds("tip-got-it").expect("the tip's button");
    cx.simulate_click(button.center(), Default::default());
    settle(&view, &mut cx);
    scroll(&mut cx, "file-list", 60.);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), None);
}

#[gpui::test]
fn tip_short_folder_scroll_does_nothing(cx: &mut TestAppContext) {
    let root = fixture("tip-short");
    let (view, mut cx) = open(cx, &root);
    scroll(&mut cx, "file-list", 60.);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), None, "three entries are not a long folder");
}

#[gpui::test]
fn tip_using_the_key_retires_it(cx: &mut TestAppContext) {
    let root = long_fixture("tip-key", 250);
    let (view, mut cx) = open(cx, &root);
    scroll(&mut cx, "file-list", 60.);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), Some(TipId::Filter));

    cx.simulate_keystrokes("/");
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), None, "using / closes the tip");
    assert!(tips_json(&cx).contains("\"filter\""), "and retires it");
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
}

#[gpui::test]
fn tip_scroll_in_column_view(cx: &mut TestAppContext) {
    let root = long_fixture("tip-columns", 250);
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3");
    settle(&view, &mut cx);
    scroll(&mut cx, "column-0", 60.);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), Some(TipId::Filter), "any column's scroll counts");
}

#[gpui::test]
fn tip_two_zero_match_filters_suggest_search(cx: &mut TestAppContext) {
    let root = fixture("tip-two-filters");
    let (view, mut cx) = open(cx, &root);
    for query in ["zzz", "qqq"] {
        cx.simulate_keystrokes("/");
        settle(&view, &mut cx);
        cx.simulate_input(query);
        settle(&view, &mut cx);
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
    }
    assert_eq!(tip(&view, &cx), Some(TipId::Search), "two zero-match filters suggest search");
    assert_eq!(
        view.read_with(&cx, |_, cx| crate::tips::key_text(TipId::Search, cx)),
        Some("⌘F".into()),
        "the tip shows the live key"
    );
}

#[gpui::test]
fn tip_zero_match_streak_resets_on_a_match(cx: &mut TestAppContext) {
    let root = fixture("tip-streak");
    let (view, mut cx) = open(cx, &root);
    for query in ["zzz", "al", "qqq"] {
        cx.simulate_keystrokes("/");
        settle(&view, &mut cx);
        cx.simulate_input(query);
        settle(&view, &mut cx);
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
    }
    assert_eq!(tip(&view, &cx), None, "a filter with matches resets the streak");
}

#[gpui::test]
fn tip_search_key_retires_it(cx: &mut TestAppContext) {
    let root = fixture("tip-search-key");
    let (view, mut cx) = open(cx, &root);
    for query in ["zzz", "qqq"] {
        cx.simulate_keystrokes("/");
        settle(&view, &mut cx);
        cx.simulate_input(query);
        settle(&view, &mut cx);
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
    }
    assert_eq!(tip(&view, &cx), Some(TipId::Search));

    cx.simulate_keystrokes("cmd-f");
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), None, "using ⌘F closes the tip");
    assert!(tips_json(&cx).contains("\"search\""), "and retires it");
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);

    for query in ["zzz", "qqq"] {
        cx.simulate_keystrokes("/");
        settle(&view, &mut cx);
        cx.simulate_input(query);
        settle(&view, &mut cx);
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
    }
    assert_eq!(tip(&view, &cx), None, "no second suggestion");
}

#[gpui::test]
fn tip_go_to_folder_for_a_recent_folder_suggests_the_palette(cx: &mut TestAppContext) {
    let root = fixture("tip-goto");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-down");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"), "visited alpha");
    cx.simulate_keystrokes("cmd-up");
    settle(&view, &mut cx);

    let recents = cx.read(|cx| cx.global::<Ui>().palette_recents.clone());
    assert!(recents.iter().any(|p| *p == root.join("alpha")), "alpha is recent: {recents:?}");
    cx.simulate_keystrokes("cmd-shift-g cmd-a");
    cx.simulate_input(&root.join("alpha").display().to_string());
    settle(&view, &mut cx);
    assert!(
        view.read_with(&cx, |w, _| w.palette.as_ref().is_some_and(|p| p.path.is_some())),
        "the typed path lists its parent"
    );
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("alpha"));
    assert_eq!(tip(&view, &cx), Some(TipId::GoTo), "a typed path for a recent folder suggests ⌘K");
    assert_eq!(view.read_with(&cx, |_, cx| crate::tips::key_text(TipId::GoTo, cx)), Some("⌘K".into()));
}

#[gpui::test]
fn tip_go_to_folder_for_a_new_folder_does_nothing(cx: &mut TestAppContext) {
    let root = fixture("tip-goto-new");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-shift-g cmd-a");
    cx.simulate_input(&root.join("beta").display().to_string());
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(dir(&view, &cx), root.join("beta"));
    assert_eq!(tip(&view, &cx), None, "a folder the palette did not know teaches nothing");
}

#[gpui::test]
fn tip_palette_key_retires_it(cx: &mut TestAppContext) {
    let root = fixture("tip-goto-key");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-down");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-up");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-shift-g cmd-a");
    cx.simulate_input(&root.join("alpha").display().to_string());
    settle(&view, &mut cx);
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), Some(TipId::GoTo));

    cx.simulate_keystrokes("cmd-k");
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), None, "using ⌘K closes the tip");
    assert!(tips_json(&cx).contains("\"go-to\""), "and retires it");
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
}

#[gpui::test]
fn tip_state_survives_reload(cx: &mut TestAppContext) {
    let root = long_fixture("tip-reload", 250);
    let (view, mut cx) = open(cx, &root);
    scroll(&mut cx, "file-list", 60.);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), Some(TipId::Filter));
    let button = cx.debug_bounds("tip-got-it").expect("the tip's button");
    cx.simulate_click(button.center(), Default::default());
    settle(&view, &mut cx);
    assert!(tips_json(&cx).contains("\"filter\""));

    cx.update(|_, cx| crate::tips::reload_for_tests(cx));
    scroll(&mut cx, "file-list", 60.);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), None, "the retired set survives a relaunch");

    // The relaunch cleared the cooldown, so a never-shown tip can still appear.
    for query in ["zzz", "qqq"] {
        cx.simulate_keystrokes("/");
        settle(&view, &mut cx);
        cx.simulate_input(query);
        settle(&view, &mut cx);
        cx.simulate_keystrokes("escape");
        settle(&view, &mut cx);
    }
    assert_eq!(tip(&view, &cx), Some(TipId::Search));
}

#[gpui::test]
fn tip_filter_dropped_by_navigation_counts(cx: &mut TestAppContext) {
    let root = fixture("tip-filter-nav");
    let (view, mut cx) = open(cx, &root);
    for _ in 0..2 {
        cx.simulate_keystrokes("/");
        settle(&view, &mut cx);
        cx.simulate_input("zzz");
        settle(&view, &mut cx);
        cx.simulate_keystrokes("cmd-up");
        settle(&view, &mut cx);
    }
    assert_eq!(tip(&view, &cx), Some(TipId::Search), "a filter dropped by navigation ends the session too");
}

#[gpui::test]
fn tip_closes_when_a_tab_is_clicked(cx: &mut TestAppContext) {
    let root = long_fixture("tip-tab-click", 250);
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-t");
    settle(&view, &mut cx);
    scroll(&mut cx, "file-list", 60.);
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), Some(TipId::Filter));

    let bounds = cx.debug_bounds("tab-0").expect("other tab on screen");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    settle(&view, &mut cx);
    assert_eq!(tip(&view, &cx), None, "switching tabs by click closes the tip");
    assert_eq!(view.read_with(&cx, |_, cx| crate::tips::owner(cx)), None, "and frees it app-wide");
}

#[gpui::test]
fn cursor_glide_sits_on_the_cursor_row(cx: &mut TestAppContext) {
    let root = long_fixture("glide", 50);
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "full");
    cx.simulate_keystrokes("down down");
    settle(&view, &mut cx);
    for _ in 0..20 {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
        cx.run_until_parked();
    }
    let glide = cx.debug_bounds("glide").expect("the glide fill");
    let row = cx.debug_bounds("row-f002.txt").expect("the cursor row");
    assert!((glide.top() - row.top()).abs() <= px(1.), "glide {glide:?} vs row {row:?}");
}

#[gpui::test]
fn cursor_glide_stays_on_its_row_while_the_trackpad_scrolls(cx: &mut TestAppContext) {
    let root = long_fixture("glide-scroll", 80);
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "full");
    cx.simulate_keystrokes("down down down down down");
    settle(&view, &mut cx);
    let frames = |cx: &mut VisualTestContext, n: usize| {
        for _ in 0..n {
            cx.executor().advance_clock(std::time::Duration::from_millis(16));
            cx.update(|window, cx| {
                window.simulate_next_frame(cx);
            });
            cx.run_until_parked();
        }
    };
    frames(&mut cx, 30);
    let before = selected(&view, &cx);
    assert_eq!(before.as_deref(), Some("f005.txt"));
    let row = cx.debug_bounds("row-f005.txt").expect("the cursor row");
    // A trackpad scroll: precise pixel deltas, a few frames apart.
    for _ in 0..4 {
        cx.simulate_event(gpui::ScrollEvent {
            position: row.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10.))),
            touch_phase: gpui::TouchPhase::Moved,
            ..Default::default()
        });
        frames(&mut cx, 1);
        let glide = cx.debug_bounds("glide").expect("the glide fill");
        let row = cx.debug_bounds("row-f005.txt").expect("the cursor row");
        assert!((glide.top() - row.top()).abs() <= px(1.), "mid-scroll: glide {glide:?} vs row {row:?}");
    }
    assert_eq!(selected(&view, &cx), before, "scrolling never moves the cursor");
    let moved = cx.debug_bounds("row-f005.txt").expect("the cursor row");
    assert!(moved.top() < row.top(), "the view scrolled: {moved:?} vs {row:?}");
}

#[gpui::test]
fn cursor_glide_stays_put_when_a_short_list_cannot_scroll(cx: &mut TestAppContext) {
    let root = long_fixture("glide-short", 6);
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "full");
    cx.simulate_keystrokes("down down");
    settle(&view, &mut cx);
    let frames = |cx: &mut VisualTestContext, n: usize| {
        for _ in 0..n {
            cx.executor().advance_clock(std::time::Duration::from_millis(16));
            cx.update(|window, cx| {
                window.simulate_next_frame(cx);
            });
            cx.run_until_parked();
        }
    };
    frames(&mut cx, 30);
    let row = cx.debug_bounds("row-f002.txt").expect("the cursor row");
    // Trackpad scrolls both ways on a list that fits: nothing may move.
    for dy in [-15., -15., 40., 40.] {
        cx.simulate_event(gpui::ScrollEvent {
            position: row.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(dy))),
            touch_phase: gpui::TouchPhase::Moved,
            ..Default::default()
        });
        frames(&mut cx, 1);
        let glide = cx.debug_bounds("glide").expect("the glide fill");
        let now = cx.debug_bounds("row-f002.txt").expect("the cursor row");
        assert_eq!(now.top(), row.top(), "the list cannot scroll");
        assert!((glide.top() - now.top()).abs() <= px(1.), "after {dy}: glide {glide:?} vs row {now:?}");
    }
}

fn disclose(mode: &str, cx: &mut TestAppContext) -> (Entity<CrowWindow>, VisualTestContext, PathBuf) {
    let root = fixture(&format!("disclose-{mode}"));
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, mode);
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"));
    cx.simulate_keystrokes("right");
    settle(&view, &mut cx);
    assert_eq!(names(&view, &cx), ["alpha", "inner", "beta", "notes.txt"], "{mode}: alpha is open");
    (view, cx, root)
}

#[gpui::test]
fn opening_a_folder_in_full_reveals_its_rows_and_slides_the_rest_down(cx: &mut TestAppContext) {
    use std::time::Duration;
    let (view, cx, root) = disclose("full", cx);
    let alpha = root.join("alpha");
    let probe =
        |ix, ms| pane(&view, &cx).read_with(&cx, |p, cx| p.disclosure_probe(&alpha, ix, Duration::from_millis(ms), cx));
    let (beta_shift, _, turn) = probe(2, 0).expect("a disclosure for alpha");
    assert_eq!(beta_shift, -1.0, "beta starts where inner lands");
    assert_eq!(turn, Some(0.0), "the chevron starts closed");
    let (_, inner_opacity, _) = probe(1, 0).expect("disclosure");
    assert_eq!(inner_opacity, 0.0, "inner shows as the gap opens");
    let (beta_mid, _, turn_mid) = probe(2, 60).expect("disclosure");
    assert!(beta_mid > -1.0 && beta_mid < 0.0, "beta slides down: {beta_mid}");
    let inner_mid = probe(1, 60).expect("disclosure").1;
    assert!(inner_mid > 0.0, "inner has started to show: {inner_mid}");
    assert!(turn_mid.is_some_and(|t| t > 0.0 && t < 1.0), "the chevron turns: {turn_mid:?}");
    assert_eq!(probe(2, 400).expect("disclosure"), (0.0, 1.0, None), "settled");
    assert!(pane(&view, &cx).read_with(&cx, |p, cx| p.animating(cx)), "frames keep coming while it moves");
}

#[gpui::test]
fn closing_a_folder_in_full_slides_the_rows_below_up(cx: &mut TestAppContext) {
    use std::time::Duration;
    let (view, mut cx, root) = disclose("full", cx);
    cx.simulate_keystrokes("left");
    settle(&view, &mut cx);
    assert_eq!(names(&view, &cx), ["alpha", "beta", "notes.txt"], "alpha is closed");
    let alpha = root.join("alpha");
    let probe =
        |ix, ms| pane(&view, &cx).read_with(&cx, |p, cx| p.disclosure_probe(&alpha, ix, Duration::from_millis(ms), cx));
    let (beta_shift, _, turn) = probe(1, 0).expect("a disclosure for alpha");
    assert_eq!(beta_shift, 1.0, "beta starts where it was");
    assert_eq!(turn, Some(1.0), "the chevron starts open");
    assert_eq!(probe(0, 0).expect("disclosure").0, 0.0, "alpha itself stays");
    assert_eq!(probe(1, 400).expect("disclosure"), (0.0, 1.0, None), "settled");
}

#[gpui::test]
fn opening_a_folder_in_low_fades_without_moving(cx: &mut TestAppContext) {
    use std::time::Duration;
    let (view, cx, root) = disclose("low", cx);
    let alpha = root.join("alpha");
    let probe =
        |ix, ms| pane(&view, &cx).read_with(&cx, |p, cx| p.disclosure_probe(&alpha, ix, Duration::from_millis(ms), cx));
    let (_, inner_opacity, turn) = probe(1, 40).expect("a disclosure for alpha");
    assert!(inner_opacity > 0.0 && inner_opacity < 1.0, "inner fades in: {inner_opacity}");
    assert_eq!(turn, None, "Low turns the chevron at once");
    assert_eq!(probe(2, 0).expect("disclosure").0, 0.0, "Low moves nothing");
}

#[gpui::test]
fn opening_a_folder_with_motion_off_changes_in_one_frame(cx: &mut TestAppContext) {
    let (view, cx, root) = disclose("off", cx);
    let alpha = root.join("alpha");
    let probe = pane(&view, &cx).read_with(&cx, |p, cx| p.disclosure_probe(&alpha, 1, std::time::Duration::ZERO, cx));
    assert_eq!(probe, None, "off records nothing");
    assert!(!pane(&view, &cx).read_with(&cx, |p, cx| p.animating(cx)), "and asks for no frames");
}

#[gpui::test]
fn cursor_glide_lags_at_most_one_row(cx: &mut TestAppContext) {
    let root = long_fixture("glide-lag", 50);
    let (_view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "full");
    // Hold ↓ at the usual repeat: one row every 30 ms.
    for _ in 0..5 {
        cx.simulate_keystrokes("down");
        cx.executor().advance_clock(std::time::Duration::from_millis(30));
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
        cx.run_until_parked();
    }
    let glide = cx.debug_bounds("glide").expect("the glide fill");
    let row = cx.debug_bounds("row-f005.txt").expect("the cursor row");
    let lag = f32::from(row.top()) - f32::from(glide.top());
    assert!(lag.abs() <= crate::pane::ROW_HEIGHT + 1.0, "the fill lags {lag} px: {glide:?} vs {row:?}");
}

#[gpui::test]
fn tip_sits_above_the_bar_at_the_right_edge(cx: &mut TestAppContext) {
    let root = long_fixture("tip-layout", 250);
    let (view, mut cx) = open(cx, &root);
    set_motion(&mut cx, "off");
    scroll(&mut cx, "file-list", 60.);
    settle(&view, &mut cx);
    let tip = cx.debug_bounds("tip").expect("the tip is laid out");
    let viewport = cx.update(|window, _| window.viewport_size());
    assert_eq!(tip.right(), viewport.width - px(12.), "12 px from the right edge");
    assert_eq!(tip.bottom(), viewport.height - px(crate::bottom_bar::HEIGHT + 8.), "8 px above the bottom bar");
    assert!(tip.size.width >= px(280.) && tip.size.width <= px(440.), "{:?}", tip.size);
}

// ----- click on empty space -----

fn list_focused(view: &Entity<CrowWindow>, cx: &mut VisualTestContext) -> bool {
    let focus = pane(view, cx).read_with(cx, |p, _| p.focus_handle().clone());
    cx.update(|window, _| focus.is_focused(window))
}

/// A point in the list body below the last row.
fn below_rows(cx: &mut VisualTestContext, last_row: &'static str) -> gpui::Point<gpui::Pixels> {
    let row = cx.debug_bounds(last_row).unwrap_or_else(|| panic!("{last_row} is on screen"));
    point(row.center().x, row.bottom() + px(40.))
}

#[gpui::test]
fn click_below_rows_clears_the_selection(cx: &mut TestAppContext) {
    let root = fixture("empty-click-list");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down shift-down");
    assert_eq!(selected_names(&view, &cx), ["beta", "notes.txt"]);

    let empty = below_rows(&mut cx, "row-notes.txt");
    cx.simulate_click(empty, gpui::Modifiers::default());
    assert!(selected_names(&view, &cx).is_empty(), "a click on empty space deselects");
    assert_eq!(selected(&view, &cx), None, "…and leaves no cursor");
    assert!(list_focused(&view, &mut cx), "focus stays on the list");

    cx.simulate_keystrokes("down");
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "↓ starts again from the first row");
}

#[gpui::test]
fn escape_clears_the_selection_when_nothing_else_is_open(cx: &mut TestAppContext) {
    let root = fixture("escape-deselect");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down");
    assert_eq!(selected(&view, &cx).as_deref(), Some("beta"));

    cx.simulate_keystrokes("escape");
    assert_eq!(selected(&view, &cx), None, "Escape clears the cursor");
    assert!(selected_names(&view, &cx).is_empty(), "and the selection");

    // The filter owns the first Escape; the next one clears the selection.
    cx.simulate_keystrokes("down /");
    cx.simulate_input("a");
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    assert!(!pane(&view, &cx).read_with(&cx, |p, _| p.is_filtering()), "Escape closed the filter");
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "the selection survived the filter close");
    cx.simulate_keystrokes("escape");
    assert_eq!(selected(&view, &cx), None, "the next Escape cleared it");
}

#[gpui::test]
fn escape_cancels_a_pending_vim_mark_before_deselecting(cx: &mut TestAppContext) {
    let root = fixture("escape-vim-mark");
    let (view, mut cx) = open(cx, &root);
    use_vim(&view, &mut cx);
    cx.simulate_keystrokes("down m");
    assert_eq!(pane(&view, &cx).read_with(&cx, |p, _| p.pending_hint()), Some("Mark this folder: type a letter"));

    cx.simulate_keystrokes("escape");
    assert_eq!(pane(&view, &cx).read_with(&cx, |p, _| p.pending_hint()), None, "Escape cancelled the mark");
    assert_eq!(selected(&view, &cx).as_deref(), Some("beta"), "and kept the selection");

    cx.simulate_keystrokes("escape");
    assert_eq!(selected(&view, &cx), None, "the next Escape clears the selection");
}

#[gpui::test]
fn click_in_the_row_gutter_clears_the_selection(cx: &mut TestAppContext) {
    let root = fixture("empty-click-gutter");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down");
    let row = cx.debug_bounds("row-beta").expect("beta row");
    cx.simulate_click(point(row.left() - px(4.), row.center().y), gpui::Modifiers::default());
    assert!(selected_names(&view, &cx).is_empty(), "the gutter beside a row is empty space");
}

#[gpui::test]
fn click_on_a_row_still_selects_it(cx: &mut TestAppContext) {
    let root = fixture("empty-click-row");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down");
    click_selector(&mut cx, "row-notes.txt");
    assert_eq!(selected_names(&view, &cx), ["notes.txt"]);
    // The row's press must not leave the next empty-space click ignored.
    let empty = below_rows(&mut cx, "row-notes.txt");
    cx.simulate_click(empty, gpui::Modifiers::default());
    assert!(selected_names(&view, &cx).is_empty());
}

#[gpui::test]
fn cmd_or_shift_click_on_empty_space_keeps_the_selection(cx: &mut TestAppContext) {
    let root = fixture("empty-click-modifiers");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down shift-down");
    let empty = below_rows(&mut cx, "row-notes.txt");
    cx.simulate_click(empty, gpui::Modifiers::command());
    assert_eq!(selected_names(&view, &cx), ["beta", "notes.txt"], "Cmd-click on empty space");
    cx.simulate_click(empty, gpui::Modifiers::shift());
    assert_eq!(selected_names(&view, &cx), ["beta", "notes.txt"], "Shift-click on empty space");
}

#[gpui::test]
fn click_on_empty_space_in_a_column_closes_the_columns_right_of_it(cx: &mut TestAppContext) {
    let root = fixture("empty-click-columns");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3 right");
    settle(&view, &mut cx);
    // alpha is selected in column 0 and its contents are column 1, with focus there and
    // its first item (the folder inner) selected, so inner's contents are column 2.
    let count = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, _| p.folder_count());
    let focused = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, _| p.focused_index());
    assert_eq!((count(&cx), focused(&cx)), (3, 1));

    let empty = below_rows(&mut cx, "column-1-row-inner");
    cx.simulate_click(empty, gpui::Modifiers::default());
    settle(&view, &mut cx);
    assert_eq!((count(&cx), focused(&cx)), (2, 1), "inner's column closes; alpha's stays focused");
    assert!(selected_names(&view, &cx).is_empty());

    let empty = below_rows(&mut cx, "column-0-row-notes.txt");
    cx.simulate_click(empty, gpui::Modifiers::default());
    settle(&view, &mut cx);
    assert_eq!(focused(&cx), 0, "the clicked column takes focus");
    assert_eq!(count(&cx), 1, "the columns to its right close");
    assert!(selected_names(&view, &cx).is_empty(), "nothing is selected in it");
    assert!(list_focused(&view, &mut cx));
    cx.simulate_keystrokes("down");
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"));
}

#[gpui::test]
fn click_on_empty_space_commits_a_rename(cx: &mut TestAppContext) {
    let root = fixture("rename-empty-click");
    let (view, mut cx) = open(cx, &root);
    click_selector(&mut cx, "row-notes.txt");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("renamed");
    let empty = below_rows(&mut cx, "row-notes.txt");
    cx.simulate_click(empty, gpui::Modifiers::default());
    settle(&view, &mut cx);
    assert!(!pane(&view, &cx).read_with(&cx, |p, _| p.is_renaming()), "the rename closed");
    assert!(root.join("renamed.txt").exists(), "and was applied");
    assert!(!root.join("notes.txt").exists());
}

#[gpui::test]
fn click_inside_the_rename_field_keeps_it_open(cx: &mut TestAppContext) {
    let root = fixture("rename-field-click");
    let (view, mut cx) = open(cx, &root);
    click_selector(&mut cx, "row-notes.txt");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("renamed");
    // Render the field as focused so Escape reaches it.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let field = cx.debug_bounds("row-name-notes.txt").expect("the rename field").center();
    cx.simulate_click(field, gpui::Modifiers::default());
    settle(&view, &mut cx);
    assert!(pane(&view, &cx).read_with(&cx, |p, _| p.is_renaming()), "clicking the field keeps the rename");
    assert!(root.join("notes.txt").exists(), "and does not apply it");
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    assert!(!pane(&view, &cx).read_with(&cx, |p, _| p.is_renaming()));
}

#[gpui::test]
fn rename_field_in_the_list_has_room_for_its_text(cx: &mut TestAppContext) {
    let root = fixture("rename-field-width");
    let (view, mut cx) = open(cx, &root);
    click_selector(&mut cx, "row-notes.txt");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("renamed");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let field = cx.debug_bounds("row-name-notes.txt").expect("the rename field");
    assert!(field.size.width > px(40.), "the field is wide enough for the typed name; got {:?}", field.size);
    assert!(pane(&view, &cx).read_with(&cx, |p, _| p.is_renaming()), "the rename is still open");
}

// ----- context menu -----

use crate::palette::CommandId;

fn right_click(cx: &mut VisualTestContext, at: gpui::Point<gpui::Pixels>) {
    cx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::default());
}

fn row_center(cx: &mut VisualTestContext, selector: &'static str) -> gpui::Point<gpui::Pixels> {
    cx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} is on screen")).center()
}

fn open_menu(view: &Entity<CrowWindow>, cx: &VisualTestContext) -> Option<crate::context_menu::MenuKind> {
    pane(view, cx).read_with(cx, |p, _| p.context_menu())
}

/// Pick the menu line running `id`, as a click on it would.
fn choose(view: &Entity<CrowWindow>, cx: &mut VisualTestContext, id: CommandId) {
    let kind = open_menu(view, cx).expect("a menu is open");
    let ix = crate::context_menu::slots(kind).iter().position(|slot| *slot == Some(id)).expect("in the menu");
    let pane = pane(view, cx);
    pane.update_in(cx, |p, window, cx| p.choose_context_item(Some(ix), window, cx));
    cx.run_until_parked();
}

#[gpui::test]
fn right_click_on_an_unselected_row_selects_it_first(cx: &mut TestAppContext) {
    use crate::context_menu::MenuKind;
    let root = fixture("context-row");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down shift-down");
    let alpha = row_center(&mut cx, "row-alpha");
    right_click(&mut cx, alpha);
    assert_eq!(selected_names(&view, &cx), ["alpha"], "Finder selects the row it opens the menu for");
    assert_eq!(open_menu(&view, &cx), Some(MenuKind::Items));
    assert!(list_focused(&view, &mut cx));
}

#[gpui::test]
fn right_click_inside_the_selection_keeps_it(cx: &mut TestAppContext) {
    use crate::context_menu::MenuKind;
    let root = fixture("context-selection");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down shift-down");
    let beta = row_center(&mut cx, "row-beta");
    right_click(&mut cx, beta);
    assert_eq!(selected_names(&view, &cx), ["beta", "notes.txt"]);
    assert_eq!(open_menu(&view, &cx), Some(MenuKind::Items));

    choose(&view, &mut cx, CommandId::MoveToTrash);
    settle(&view, &mut cx);
    assert!(!root.join("beta").exists() && !root.join("notes.txt").exists(), "both went to the Trash");
    assert!(root.join("alpha").exists());
    assert_eq!(open_menu(&view, &cx), None, "the menu is closed");
}

#[gpui::test]
fn ctrl_click_opens_the_menu_like_a_right_click(cx: &mut TestAppContext) {
    use crate::context_menu::MenuKind;
    let root = fixture("context-ctrl");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down");
    let notes = row_center(&mut cx, "row-notes.txt");
    cx.simulate_click(notes, gpui::Modifiers::control());
    assert_eq!(selected_names(&view, &cx), ["notes.txt"]);
    assert_eq!(open_menu(&view, &cx), Some(MenuKind::Items));

    // Dismissing the menu runs nothing.
    let pane = pane(&view, &cx);
    pane.update_in(&mut cx, |p, window, cx| p.choose_context_item(None, window, cx));
    cx.run_until_parked();
    assert_eq!(open_menu(&view, &cx), None);
    assert_eq!(selected_names(&view, &cx), ["notes.txt"]);

    let empty = below_rows(&mut cx, "row-notes.txt");
    cx.simulate_click(empty, gpui::Modifiers::control());
    assert_eq!(open_menu(&view, &cx), Some(MenuKind::Folder), "Ctrl-click on empty space");
    assert!(selected_names(&view, &cx).is_empty());
}

#[gpui::test]
fn right_click_on_empty_space_offers_new_folder(cx: &mut TestAppContext) {
    use crate::context_menu::MenuKind;
    let root = fixture("context-empty");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down");
    let empty = below_rows(&mut cx, "row-notes.txt");
    right_click(&mut cx, empty);
    assert!(selected_names(&view, &cx).is_empty(), "empty space deselects, as in Finder");
    assert_eq!(open_menu(&view, &cx), Some(MenuKind::Folder));

    choose(&view, &mut cx, CommandId::NewFolder);
    settle(&view, &mut cx);
    cx.run_until_parked();
    let renaming = pane(&view, &cx).read_with(&cx, |p, cx| p.rename_text(cx));
    assert_eq!(renaming.as_deref(), Some("untitled folder"), "created, and its rename started as with ⇧⌘N");
    assert!(root.join("untitled folder").is_dir());
}

#[gpui::test]
fn right_click_in_a_column_works_on_that_column(cx: &mut TestAppContext) {
    use crate::context_menu::MenuKind;
    let root = fixture("context-columns");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-3 right");
    settle(&view, &mut cx);
    let count = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, _| p.folder_count());
    let focused = |cx: &VisualTestContext| pane(&view, cx).read_with(cx, |p, _| p.focused_index());

    let notes = row_center(&mut cx, "column-0-row-notes.txt");
    right_click(&mut cx, notes);
    settle(&view, &mut cx);
    assert_eq!((focused(&cx), count(&cx)), (0, 1), "the first column takes focus; a file opens no column");
    assert_eq!(selected_names(&view, &cx), ["notes.txt"]);
    assert_eq!(open_menu(&view, &cx), Some(MenuKind::Items));

    choose(&view, &mut cx, CommandId::Duplicate);
    settle(&view, &mut cx);
    assert!(root.join("notes copy.txt").exists(), "Duplicate ran on the column's selection");

    let empty = below_rows(&mut cx, "column-0-row-notes.txt");
    right_click(&mut cx, empty);
    assert_eq!(open_menu(&view, &cx), Some(MenuKind::Folder));
    assert!(selected_names(&view, &cx).is_empty());
}

#[gpui::test]
fn copy_path_puts_the_selected_paths_on_the_clipboard(cx: &mut TestAppContext) {
    let root = fixture("copy-path");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down shift-down cmd-alt-c");
    let text = cx.read(|cx| crate::effects::recording(cx).and_then(|recorder| recorder.text()));
    let expected = format!("{}\n{}", root.join("beta").display(), root.join("notes.txt").display());
    assert_eq!(text.as_deref(), Some(expected.as_str()), "⌥⌘C copies one path per line");
    let message = cx.read(|cx| cx.global::<Operations>().message.clone());
    assert_eq!(message.map(|(text, _)| text.to_string()).as_deref(), Some("Copied 2 paths"));

    // With nothing selected it copies the folder's own path, as Finder does.
    let empty = below_rows(&mut cx, "row-notes.txt");
    cx.simulate_click(empty, gpui::Modifiers::default());
    cx.simulate_keystrokes("cmd-alt-c");
    let text = cx.read(|cx| crate::effects::recording(cx).and_then(|recorder| recorder.text()));
    assert_eq!(text, Some(root.display().to_string()));

    // And from the menu.
    let alpha = row_center(&mut cx, "row-alpha");
    right_click(&mut cx, alpha);
    choose(&view, &mut cx, CommandId::CopyPath);
    let text = cx.read(|cx| crate::effects::recording(cx).and_then(|recorder| recorder.text()));
    assert_eq!(text, Some(root.join("alpha").display().to_string()));

    // The empty-space menu offers it too, and copies the shown folder.
    let empty = below_rows(&mut cx, "row-notes.txt");
    right_click(&mut cx, empty);
    choose(&view, &mut cx, CommandId::CopyPath);
    let text = cx.read(|cx| crate::effects::recording(cx).and_then(|recorder| recorder.text()));
    assert_eq!(text, Some(root.display().to_string()), "empty space copies the folder shown");
}

#[gpui::test]
fn context_menu_lines_use_the_menu_bar_names_and_keys(cx: &mut TestAppContext) {
    use crate::context_menu::{MenuKind, MenuState, entries};
    use crow_macos::{ContextMenuEntry, MenuShortcut};
    let root = fixture("context-lines");
    let (view, mut cx) = open(cx, &root);
    let line = |cx: &mut VisualTestContext, title: &str| {
        let state = MenuState { selected: 1, selected_folders: 0, can_paste: false };
        cx.update(|_, cx| entries(MenuKind::Items, state, crate::keys::KeyScope::List, cx))
            .into_iter()
            .find(|entry| matches!(entry, ContextMenuEntry::Item { title: t, .. } if t == title))
            .unwrap_or_else(|| panic!("{title} is in the menu"))
    };
    let shortcut =
        |key: &str, cmd: bool, alt: bool| Some(MenuShortcut { key: key.into(), cmd, alt, ..Default::default() });
    assert_eq!(
        line(&mut cx, "Move to Trash"),
        ContextMenuEntry::Item {
            title: "Move to Trash".into(),
            shortcut: shortcut("backspace", true, false),
            enabled: true
        }
    );
    assert_eq!(
        line(&mut cx, "Paste"),
        ContextMenuEntry::Item { title: "Paste".into(), shortcut: shortcut("v", true, false), enabled: false },
        "nothing to paste"
    );
    assert_eq!(
        line(&mut cx, "Copy Path"),
        ContextMenuEntry::Item { title: "Copy Path".into(), shortcut: shortcut("c", true, true), enabled: true }
    );
    // The Vim layer's `y y` can't be shown in a native menu; ⌘C still is.
    use_vim(&view, &mut cx);
    assert_eq!(
        line(&mut cx, "Copy"),
        ContextMenuEntry::Item { title: "Copy".into(), shortcut: shortcut("c", true, false), enabled: true }
    );
}

// ----- fuzzer runner -----

/// Recreate the H1 tree from scratch: the usual fixture plus a few hundred files,
/// then the edge kinds. Every seed and every shrink trial starts here, so one
/// run's trash, rename or mode-000 folder cannot change the next one.
fn reset_tree(root: &Path, files: usize, edge: &EdgeTree, seed: u64) {
    crow_fs::testing::remove_tree(root).expect("reset fuzz tree");
    std::fs::create_dir_all(root.join("alpha/inner")).expect("fuzz tree");
    std::fs::create_dir_all(root.join("beta")).expect("fuzz tree");
    std::fs::write(root.join("notes.txt"), "hello\n").expect("fuzz tree");
    for i in 0..files {
        std::fs::write(root.join(format!("file-{i:03}.txt")), "x").expect("fuzz file");
    }
    for i in 0..20 {
        std::fs::create_dir_all(root.join(format!("dir-{i:02}/inner"))).expect("fuzz dir");
    }
    if !edge.is_empty() {
        crow_fs::testing::make_tree(root, edge, seed).expect("edge tree");
    }
}

fn fuzz_sidebar_toml(root: &Path) -> String {
    format!(
        "[[group]]\nname = \"Favorites\"\nitems = [\n  {{ path = \"{}\" }},\n  {{ path = \"{}\" }},\n]\n[recent]\nshow = false\n[locations]\nshow = false\n",
        root.join("alpha").display(),
        root.join("beta").display()
    )
}

fn fuzz_tree(name: &str, files: usize) -> PathBuf {
    let root = std::env::temp_dir().join(format!("crow-app-{name}-{}", std::process::id()));
    reset_tree(&root, files, &EdgeTree::default(), 0);
    root.canonicalize().expect("canonical fuzz tree")
}

/// What a run builds and how it behaves. The seed stays a separate argument so
/// the shrinker can replay a candidate under the same tree and churn setting.
#[derive(Clone)]
struct FuzzOptions {
    files: usize,
    edge: EdgeTree,
    churn: bool,
}

impl FuzzOptions {
    fn new(files: usize, edge: EdgeTree, churn: bool) -> Self {
        FuzzOptions { files, edge, churn }
    }

    fn without_churn(&self) -> Self {
        FuzzOptions { churn: false, ..self.clone() }
    }
}

fn close_all_windows(cx: &mut TestAppContext) {
    for _ in 0..3 {
        let handles = cx.update(|cx| cx.windows());
        if handles.is_empty() {
            return;
        }
        for handle in handles {
            handle.update(cx, |_, window, _| window.remove_window()).ok();
        }
        cx.run_until_parked();
    }
    assert!(cx.update(|cx| cx.windows().is_empty()), "windows did not close");
}

/// `debug_bounds` wants `&'static str`; fixed chrome selectors are matched here.
fn static_selector(selector: &str) -> Option<&'static str> {
    Some(match selector {
        "toolbar" => "toolbar",
        "toolbar-go-to" => "toolbar-go-to",
        "toolbar-search" => "toolbar-search",
        "path-bar" => "path-bar",
        "file-list" => "file-list",
        "table-header" => "table-header",
        "columns" => "columns",
        "preview" => "preview",
        "filter-field" => "filter-field",
        "filter-search-footer" => "filter-search-footer",
        "search-field" => "search-field",
        "search-done" => "search-done",
        "search-scope-bar" => "search-scope-bar",
        "search-skip-switch" => "search-skip-switch",
        "search-footer" => "search-footer",
        "search-skipped-link" => "search-skipped-link",
        "search-idle" => "search-idle",
        "palette-commands-token" => "palette-commands-token",
        "palette-empty" => "palette-empty",
        "why-open-file" => "why-open-file",
        "why-reset" => "why-reset",
        "why-defaults" => "why-defaults",
        "why-close" => "why-close",
        "sheet-panel" => "sheet-panel",
        "sheet-scrim" => "sheet-scrim",
        "sheet-confirm" => "sheet-confirm",
        "sheet-cancel" => "sheet-cancel",
        "theme-list" => "theme-list",
        "tip" => "tip",
        "tip-got-it" => "tip-got-it",
        _ => return None,
    })
}

fn selector_bounds(
    view: &Entity<CrowWindow>,
    selector: &str,
    cx: &mut VisualTestContext,
) -> Option<gpui::Bounds<gpui::Pixels>> {
    if let Some(bounds) = view.read_with(cx, |window, cx| window.debug_selector_bounds(selector, cx)) {
        return Some(bounds);
    }
    let selector = static_selector(selector)?;
    cx.debug_bounds(selector)
}

fn panic_text(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "panic".to_owned()
    }
}

fn apply_step(
    view: &Entity<CrowWindow>,
    step: &crate::fuzz::Step,
    tree: &Path,
    cx: &mut VisualTestContext,
) -> Result<(), crate::fuzz::Violation> {
    use crate::fuzz::Step;
    if let Step::Keys(keys) = step
        && keys == "cmd-w"
        && view.read_with(cx, |window, _| window.tab_count()) <= 1
    {
        return Ok(());
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match step {
        Step::Keys(keys) => cx.simulate_keystrokes(keys),
        Step::Type(text) => cx.simulate_input(text),
        Step::Click(selector) => {
            if let Some(bounds) = selector_bounds(view, selector, cx) {
                cx.simulate_click(bounds.center(), gpui::Modifiers::default());
            }
        }
        Step::RightClick(selector, choice) => {
            if let Some(bounds) = selector_bounds(view, selector, cx) {
                let at = bounds.center();
                let menu = |choice: Option<usize>, cx: &mut VisualTestContext| {
                    cx.update(|_, cx| {
                        crate::effects::recording(cx).expect("recording effects").set_context_menu_choice(choice)
                    })
                };
                menu(*choice, cx);
                cx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::default());
                cx.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::default());
                menu(None, cx);
            }
        }
        Step::Resize(width, height) => cx.simulate_resize(gpui::size(gpui::px(*width), gpui::px(*height))),
        Step::Settle => crate::fuzz::settle(view, cx),
        Step::AdvanceClock(ms) => cx.executor().advance_clock(Duration::from_millis(*ms)),
        Step::DeleteShownFolder => {
            let dir = view.read_with(cx, |window, cx| window.pane().read(cx).current_dir(cx));
            if dir.starts_with(tree) {
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
        Step::ReplaceShownFolder => {
            let dir = view.read_with(cx, |window, cx| window.pane().read(cx).current_dir(cx));
            if dir.starts_with(tree) {
                let _ = std::fs::remove_dir_all(&dir);
                let _ = std::fs::create_dir_all(&dir);
                for i in 0..3 {
                    let _ = std::fs::write(dir.join(format!("replaced-{i}.txt")), "replaced\n");
                }
            }
        }
    }));
    match result {
        Ok(()) => Ok(()),
        Err(payload) => Err(crate::fuzz::Violation::new("panic", panic_text(payload))),
    }
}

fn check_windows(cx: &mut VisualTestContext, deep: bool, root: Option<&Path>) -> Vec<crate::fuzz::Violation> {
    let handles = cx.update(|_, cx| cx.windows());
    let mut out = Vec::new();
    for handle in handles {
        if let Some(view) = handle.downcast::<CrowWindow>()
            && let Ok(found) =
                view.update(&mut cx.cx, |window_view, window, cx| window_view.check_invariants(window, cx, deep, root))
        {
            out.extend(found);
        } else if let Some(view) = handle.downcast::<crate::settings_window::SettingsWindow>()
            && let Ok(found) = view.update(&mut cx.cx, |settings, window, _| settings.check_invariants(window))
        {
            out.extend(found);
        }
    }
    out
}

/// A deep listing mismatch gets a bounded retry: the trash watcher and the
/// folder poll can land a few frames after the operation itself finishes.
fn check_all_windows(cx: &mut VisualTestContext, deep: bool, root: Option<&Path>) -> Vec<crate::fuzz::Violation> {
    let mut found = check_windows(cx, deep, root);
    if deep && found.iter().any(|violation| violation.check == "listing") {
        for _ in 0..25 {
            std::thread::sleep(Duration::from_millis(20));
            cx.executor().advance_clock(Duration::from_millis(50));
            cx.run_until_parked();
            found = check_windows(cx, deep, root);
            if !found.iter().any(|violation| violation.check == "listing") {
                break;
            }
        }
    }
    found
}

/// Typed paths and the palette can leave the temp tree; bring the pane back so
/// the rest of the run (and the listing check) stays in H1's tree.
fn keep_in_tree(view: &Entity<CrowWindow>, tree: &Path, cx: &mut VisualTestContext) {
    let dir = view.read_with(cx, |window, cx| window.pane().read(cx).current_dir(cx));
    if dir.starts_with(tree) {
        return;
    }
    view.update(cx, |window, cx| {
        let pane = window.pane().clone();
        pane.update(cx, |pane, cx| pane.navigate(tree.to_path_buf(), cx));
    });
    crate::fuzz::settle(view, cx);
}

fn apply_and_check(
    view: &Entity<CrowWindow>,
    step: &crate::fuzz::Step,
    tree: &Path,
    churn: Option<&ChurnHandle>,
    cx: &mut VisualTestContext,
) -> Result<(), crate::fuzz::Violation> {
    crate::fuzz::logs::reset();
    let deep = matches!(step, crate::fuzz::Step::Settle);
    // A deep listing check compares the pane to the disk: quiet the churn first.
    if deep && let Some(churn) = churn {
        churn.pause();
    }
    let result: Result<(), crate::fuzz::Violation> = (|| {
        apply_step(view, step, tree, cx)?;
        // The step ran with churn active; `keep_in_tree` may navigate back and
        // settle, and churn would keep the folder busy until the 6 s bound
        // (seed 1794851559033176001, churn on). Quiet the disk for the check.
        if !deep && let Some(churn) = churn {
            churn.pause();
        }
        keep_in_tree(view, tree, cx);
        cx.run_until_parked();
        if let Some(violation) = check_all_windows(cx, deep, Some(tree)).into_iter().next() {
            return Err(violation);
        }
        if crate::fuzz::logs::errors() > 0 {
            return Err(crate::fuzz::Violation::new("log", "an error was logged"));
        }
        Ok(())
    })();
    if let Some(churn) = churn {
        churn.resume();
    }
    // The recorder is a test observation buffer; a run does not read it, so
    // drop each step's calls instead of letting them pile up.
    cx.update(|_, cx| {
        if let Some(recorder) = crate::effects::recording(cx) {
            recorder.clear();
        }
    });
    result
}

/// Confine a fuzz run to `tree`:
/// - every file operation outside it is refused (and reported as `jail`);
/// - "home" is the tree, so Go Home, `~` and search's Home scope stay inside;
/// - a right-click's menu closes unless the step picks an item (the native
///   menu always closes; unanswered, the pane or sidebar would stay "menu open").
fn fuzz_confine(tree: &Path, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        cx.global_mut::<crate::operations::Operations>().jail = Some(tree.to_path_buf());
        cx.global_mut::<Ui>().home = tree.to_path_buf();
        crate::effects::recording(cx).expect("recording effects").set_context_menu_choice(None);
    });
}

fn replay_script(
    script: &[crate::fuzz::Step],
    tree: &Path,
    seed: u64,
    options: &FuzzOptions,
    cx: &mut TestAppContext,
) -> Result<(), crate::fuzz::Violation> {
    close_all_windows(cx);
    reset_tree(tree, options.files, &options.edge, seed);
    let (view, mut cx) = open(cx, tree);
    write_config(&view, &mut cx, "sidebar.toml", &fuzz_sidebar_toml(tree));
    fuzz_confine(tree, &mut cx);
    crate::fuzz::logs::install();
    let churn_handle = options.churn.then(|| Churn::start(tree, seed));
    for step in script {
        apply_and_check(&view, step, tree, churn_handle.as_ref(), &mut cx)?;
    }
    if let Some(churn) = &churn_handle {
        churn.pause();
    }
    crate::fuzz::settle(&view, &mut cx);
    let found = check_all_windows(&mut cx, true, Some(tree)).into_iter().next();
    drop(churn_handle);
    found.map_or(Ok(()), Err)
}

fn fuzz_run(
    seed: u64,
    steps: usize,
    tree: &Path,
    options: &FuzzOptions,
    cx: &mut TestAppContext,
) -> Result<(), Box<crate::fuzz::Failure>> {
    use crate::fuzz::{Failure, Script, StepGen, state};
    close_all_windows(cx);
    reset_tree(tree, options.files, &options.edge, seed);
    let (view, mut cx) = open(cx, tree);
    write_config(&view, &mut cx, "sidebar.toml", &fuzz_sidebar_toml(tree));
    fuzz_confine(tree, &mut cx);
    crate::fuzz::logs::install();
    let mut churn_handle = options.churn.then(|| Churn::start(tree, seed));
    let mut generator = StepGen::new(seed);
    let mut script = Vec::new();
    for index in 0..steps {
        let state = state(&view, &mut cx);
        let step = generator.next(&state);
        script.push(step.clone());
        if let Err(violation) = apply_and_check(&view, &step, tree, churn_handle.as_ref(), &mut cx) {
            // Printed before shrinking, which can take long: a killed run still names the seed.
            println!(
                "seed {seed} step {index}: {} — {} (tree {}, churn {}; shrinking…)",
                violation.check,
                violation.detail,
                options.edge.label(),
                options.churn
            );
            drop(churn_handle.take());
            let shrunk = shrink(&script, &violation, tree, seed, options, &mut cx.cx);
            let reproduces =
                !options.churn || fails_without_churn(&shrunk, &violation, tree, seed, options, &mut cx.cx);
            return Err(Box::new(Failure {
                seed,
                step: index,
                violation,
                script: shrunk,
                original: Script(script),
                tree: options.edge.spec(),
                churn: options.churn,
                reproduces_without_churn: reproduces,
            }));
        }
    }
    if let Some(churn) = &churn_handle {
        churn.pause();
    }
    crate::fuzz::settle(&view, &mut cx);
    let found = check_all_windows(&mut cx, true, Some(tree)).into_iter().next();
    drop(churn_handle);
    if let Some(violation) = found {
        println!(
            "seed {seed} step {steps}: {} — {} (tree {}, churn {}; shrinking…)",
            violation.check,
            violation.detail,
            options.edge.label(),
            options.churn
        );
        let shrunk = shrink(&script, &violation, tree, seed, options, &mut cx.cx);
        let reproduces = !options.churn || fails_without_churn(&shrunk, &violation, tree, seed, options, &mut cx.cx);
        return Err(Box::new(Failure {
            seed,
            step: steps,
            violation,
            script: shrunk,
            original: Script(script),
            tree: options.edge.spec(),
            churn: options.churn,
            reproduces_without_churn: reproduces,
        }));
    }
    Ok(())
}

fn settings_handle(cx: &mut TestAppContext) -> Option<WindowHandle<crate::settings_window::SettingsWindow>> {
    cx.update(|cx| cx.windows().into_iter().find_map(|handle| handle.downcast()))
}

fn settings_state(
    view: &Entity<crate::settings_window::SettingsWindow>,
    cx: &mut VisualTestContext,
) -> crate::fuzz::GenState {
    cx.update(|window, cx| crate::fuzz::settings_state_for(view.read(cx), window, cx))
}

/// The window's shown settings versus `settings.toml`. A write can land a few
/// turns after the control changes, so the settle step retries; this is the
/// final answer.
fn settings_file_mismatch(cx: &mut VisualTestContext) -> Option<crate::fuzz::Violation> {
    let config = cx.read_global::<Ui, _>(|ui, _| ui.config_dir.clone());
    let shown = cx.read_global::<Ui, _>(|ui, _| ui.settings.clone());
    match crow_core::load_settings(&config) {
        Ok(file) if file == shown => None,
        Ok(file) => Some(crate::fuzz::Violation::new(
            "settings file",
            format!("the window shows {shown:?} but settings.toml says {file:?}"),
        )),
        Err(error) => {
            Some(crate::fuzz::Violation::new("settings file", format!("settings.toml does not load: {error}")))
        }
    }
}

fn apply_settings_step(step: &crate::fuzz::Step, cx: &mut VisualTestContext) -> Result<(), crate::fuzz::Violation> {
    use crate::fuzz::Step;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match step {
        Step::Keys(keys) => cx.simulate_keystrokes(keys),
        Step::Type(text) => cx.simulate_input(text),
        Step::Click(selector) => {
            if let Some(bounds) = cx.update(|window, _| window.debug_bounds(selector)) {
                cx.simulate_click(bounds.center(), gpui::Modifiers::default());
            }
        }
        Step::Settle => {
            cx.run_until_parked();
            for _ in 0..40 {
                if settings_file_mismatch(cx).is_none() {
                    break;
                }
                cx.executor().advance_clock(Duration::from_millis(10));
                cx.run_until_parked();
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        Step::AdvanceClock(ms) => cx.executor().advance_clock(Duration::from_millis(*ms)),
        _ => {}
    }));
    match result {
        Ok(()) => Ok(()),
        Err(payload) => Err(crate::fuzz::Violation::new("panic", panic_text(payload))),
    }
}

fn apply_settings_and_check(
    view: &Entity<crate::settings_window::SettingsWindow>,
    step: &crate::fuzz::Step,
    cx: &mut VisualTestContext,
) -> Result<(), crate::fuzz::Violation> {
    crate::fuzz::logs::reset();
    apply_settings_step(step, cx)?;
    cx.run_until_parked();
    if let Some(violation) = cx.update(|window, cx| view.read(cx).check_invariants(window)).into_iter().next() {
        return Err(violation);
    }
    if matches!(step, crate::fuzz::Step::Settle)
        && let Some(violation) = settings_file_mismatch(cx)
    {
        return Err(violation);
    }
    if crate::fuzz::logs::errors() > 0 {
        return Err(crate::fuzz::Violation::new("log", "an error was logged"));
    }
    Ok(())
}

/// I2b: drive the Settings window as a second fuzz surface. Same generator,
/// caps and error-log recorder as H1; its own keymap and control selectors.
fn settings_run(seed: u64, steps: usize, cx: &mut TestAppContext) -> Result<(), Box<crate::fuzz::Failure>> {
    use crate::fuzz::{Failure, Script, StepGen};
    close_all_windows(cx);
    let tree = fuzz_tree(&format!("settings-{seed}"), 6);
    let (browser, mut bcx) = open(cx, &tree);
    bcx.update(|_, cx| crate::settings_window::open(cx));
    bcx.run_until_parked();
    let handle = settings_handle(&mut bcx.cx).expect("settings window");
    let mut scx = VisualTestContext::from_window(handle.into(), &bcx.cx);
    let view = handle.root(&mut scx).expect("settings root");
    scx.update(|window, cx| {
        let clear = window.draw(cx);
        clear.clear(cx);
    });
    scx.run_until_parked();
    let painted = scx.update(|window, _| window.debug_interactive_elements().expect("recorded").len());
    assert!(painted > 0, "the Settings window painted no interactive elements");
    crate::fuzz::logs::install();
    let mut generator = StepGen::new(seed);
    let mut script = Vec::new();
    let failure = |step: usize, violation: crate::fuzz::Violation, script: Vec<crate::fuzz::Step>| {
        Box::new(Failure {
            seed,
            step,
            violation,
            script: Script(script.clone()),
            original: Script(script),
            tree: "settings".to_owned(),
            churn: false,
            reproduces_without_churn: true,
        })
    };
    for index in 0..steps {
        let state = settings_state(&view, &mut scx);
        let step = generator.next_settings(&state);
        script.push(step.clone());
        if let Err(violation) = apply_settings_and_check(&view, &step, &mut scx) {
            println!("settings seed {seed} step {index}: {} — {}", violation.check, violation.detail);
            return Err(failure(index, violation, script));
        }
    }
    crate::fuzz::settle(&browser, &mut bcx);
    // The last step may have started a write; settle it like any other step.
    if let Err(violation) = apply_settings_and_check(&view, &crate::fuzz::Step::Settle, &mut scx) {
        println!("settings seed {seed} step {steps}: {} — {}", violation.check, violation.detail);
        return Err(failure(steps, violation, script));
    }
    Ok(())
}

/// The plan asks whether a churn finding still fails without churn.
fn fails_without_churn(
    script: &crate::fuzz::Script,
    target: &crate::fuzz::Violation,
    tree: &Path,
    seed: u64,
    options: &FuzzOptions,
    cx: &mut TestAppContext,
) -> bool {
    let quiet = options.without_churn();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| replay_script(&script.0, tree, seed, &quiet, cx))) {
        Ok(Err(found)) => found.check == target.check,
        Ok(Ok(())) => false,
        Err(_) => target.check == "panic",
    }
}

/// Delta debugging: drop halves, then single steps, then shorten `Type` strings.
/// Each candidate runs in a fresh window; the trial cap keeps a finding bounded.
/// A candidate counts only when it fails the same check as `target`, so the
/// shrink cannot slip to a different bug. A trial replays the whole candidate,
/// so long scripts are slow: the shrink stops after `CROW_FUZZ_SHRINK_SECS`
/// (default 60) and returns the shortest failing script found so far.
fn shrink(
    script: &[crate::fuzz::Step],
    target: &crate::fuzz::Violation,
    tree: &Path,
    seed: u64,
    options: &FuzzOptions,
    cx: &mut TestAppContext,
) -> crate::fuzz::Script {
    use crate::fuzz::{Script, Step};
    let mut best = script.to_vec();
    let trials = std::cell::Cell::new(0usize);
    let budget = std::env::var("CROW_FUZZ_SHRINK_SECS").ok().and_then(|value| value.parse().ok()).unwrap_or(60);
    let deadline = Instant::now() + Duration::from_secs(budget);
    let best_len = std::cell::Cell::new(best.len());
    let fails = |candidate: &[Step], cx: &mut TestAppContext| {
        if candidate.is_empty() || trials.get() >= 300 {
            return false;
        }
        if Instant::now() >= deadline {
            // Out of time: stop every loop below and keep the best so far.
            trials.set(300);
            println!("shrink stopped after {budget} s at {} steps", best_len.get());
            return false;
        }
        trials.set(trials.get() + 1);
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            replay_script(candidate, tree, seed, options, cx)
        })) {
            Ok(Err(found)) => found.check == target.check,
            Ok(Ok(())) => false,
            Err(_) => target.check == "panic",
        }
    };
    let mut changed = true;
    while changed && trials.get() < 300 {
        changed = false;
        let mut chunk = best.len() / 2;
        while chunk >= 1 && trials.get() < 300 {
            let mut i = 0;
            while i < best.len() && trials.get() < 300 {
                let mut candidate = best.clone();
                candidate.drain(i..(i + chunk).min(best.len()));
                if fails(&candidate, cx) {
                    best = candidate;
                    best_len.set(best.len());
                    changed = true;
                } else {
                    i += chunk;
                }
            }
            chunk /= 2;
        }
        let mut i = 0;
        while i < best.len() && trials.get() < 300 {
            let mut candidate = best.clone();
            candidate.remove(i);
            if fails(&candidate, cx) {
                best = candidate;
                best_len.set(best.len());
                changed = true;
            } else {
                i += 1;
            }
        }
        for i in 0..best.len() {
            if trials.get() >= 300 {
                break;
            }
            if let Step::Type(text) = &best[i] {
                for shorter in [String::new(), text.chars().next().map(|c| c.to_string()).unwrap_or_default()] {
                    if shorter == *text {
                        continue;
                    }
                    let mut candidate = best.clone();
                    candidate[i] = Step::Type(shorter);
                    if fails(&candidate, cx) {
                        best = candidate;
                        changed = true;
                        break;
                    }
                }
            }
        }
    }
    Script(best)
}

/// The replay entry point the printed scripts use.
fn replay(
    cx: &mut TestAppContext,
    tree: &Path,
    files: usize,
    script: crate::fuzz::Script,
) -> Vec<crate::fuzz::Violation> {
    let options = FuzzOptions::new(files, EdgeTree::default(), false);
    replay_script(&script.0, tree, 0, &options, cx).err().into_iter().collect()
}

/// The replay entry point for edge-tree and churn findings.
fn replay_with(
    cx: &mut TestAppContext,
    tree: &Path,
    files: usize,
    edge: &EdgeTree,
    seed: u64,
    churn: bool,
    script: crate::fuzz::Script,
) -> Vec<crate::fuzz::Violation> {
    let options = FuzzOptions::new(files, edge.clone(), churn);
    replay_script(&script.0, tree, seed, &options, cx).err().into_iter().collect()
}

#[gpui::test]
fn the_fuzz_jail_refuses_operations_outside_the_tree(cx: &mut TestAppContext) {
    let root = fixture("fuzz-jail");
    let (view, mut cx) = open(cx, &root);
    // Jail to alpha/: notes.txt, in the root, is outside it.
    fuzz_confine(&root.join("alpha"), &mut cx);
    click_selector(&mut cx, "row-notes.txt");
    cx.simulate_keystrokes("cmd-backspace");
    settle(&view, &mut cx);
    assert!(root.join("notes.txt").exists(), "the trash was refused");
    let found = violations(&view, &mut cx);
    assert!(found.iter().any(|violation| violation.check == "jail"), "{found:?}");
}

#[gpui::test]
fn fuzzed_right_click_runs_the_chosen_item(cx: &mut TestAppContext) {
    use crate::context_menu::{MenuKind, slots};
    use crate::fuzz::Step;
    use crate::palette::CommandId;
    let tree = fuzz_tree("fuzz-menu", 40);
    let trash = slots(MenuKind::Items).iter().position(|slot| *slot == Some(CommandId::MoveToTrash));
    let found = replay(
        cx,
        &tree,
        40,
        crate::script![
            Step::RightClick("row-file-000.txt".into(), trash),
            Step::RightClick("row-file-001.txt".into(), None)
        ],
    );
    assert!(found.is_empty(), "{found:?}");
    assert!(!tree.join("file-000.txt").exists(), "the chosen item ran");
    assert!(tree.join("file-001.txt").exists(), "a dismissed menu does nothing");
}

#[gpui::test]
fn deleting_the_shown_folder_keeps_the_invariants(cx: &mut TestAppContext) {
    use crate::fuzz::Step;
    let tree = fuzz_tree("fuzz-delete", 40);
    let found = replay(
        cx,
        &tree,
        40,
        crate::script![
            Step::Click("row-alpha".into()),
            Step::Keys("cmd-down".into()),
            Step::DeleteShownFolder,
            Step::Settle
        ],
    );
    assert!(found.is_empty(), "{found:?}");
    assert!(!tree.join("alpha").exists(), "the shown folder is gone");
}

#[gpui::test]
fn replace_shown_folder_reloads(cx: &mut TestAppContext) {
    use crate::fuzz::Step;
    let tree = fuzz_tree("fuzz-replace", 40);
    let found = replay(
        cx,
        &tree,
        40,
        crate::script![
            Step::Click("row-alpha".into()),
            Step::Keys("cmd-down".into()),
            Step::ReplaceShownFolder,
            Step::Settle
        ],
    );
    assert!(found.is_empty(), "{found:?}");
    let mut names: Vec<String> = std::fs::read_dir(tree.join("alpha"))
        .expect("the folder was recreated")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["replaced-0.txt", "replaced-1.txt", "replaced-2.txt"]);
}

/// I2's test: churn for 2 s on a 1k tree, then pause and compare the listing to
/// the disk (H1's invariant 6 on purpose).
#[gpui::test]
fn churn_run_then_settle_listing_equals_disk(cx: &mut TestAppContext) {
    let tree = fuzz_tree("fuzz-churn", 1000);
    let (view, mut cx) = open(cx, &tree);
    write_config(&view, &mut cx, "sidebar.toml", &fuzz_sidebar_toml(&tree));
    fuzz_confine(&tree, &mut cx);
    crate::fuzz::logs::install();
    let churn = Churn::start(&tree, 7);
    std::thread::sleep(Duration::from_secs(2));
    assert!(churn.operations() > 0, "churn ran");
    churn.pause();
    crate::fuzz::settle(&view, &mut cx);
    let found = check_all_windows(&mut cx, true, Some(&tree));
    assert!(found.is_empty(), "{found:?}");
}

#[gpui::test]
fn edge_tree_replay_lists_a_long_name(cx: &mut TestAppContext) {
    use crate::fuzz::Step;
    let edge = EdgeTree::single(crow_fs::testing::EdgeKind::LongName);
    let tree = fuzz_tree("fuzz-edge-replay", 40);
    let found = replay_with(cx, &tree, 40, &edge, 1, false, crate::script![Step::Settle]);
    assert!(found.is_empty(), "{found:?}");
    assert!(tree.join(format!("{}.txt", "a".repeat(251))).exists(), "the edge tree was built");
}

/// H4 #7: replacing the shown root left some old rows, because a watcher event
/// naming the root is not a child change. FSEvents delivers that event on its
/// own schedule: the bug failed 2 of 17 runs before the fix, so loop to make the
/// guard reliable rather than depending on one lucky run.
#[gpui::test]
fn replacing_the_shown_folder_reloads(cx: &mut TestAppContext) {
    use crate::fuzz::Step;
    let tree = fuzz_tree("fuzz-replace-root", 40);
    for run in 0..20 {
        let found = replay(cx, &tree, 40, crate::script![Step::ReplaceShownFolder, Step::Settle]);
        assert!(found.is_empty(), "run {run}: {found:?}");
    }
}

/// H4 #6: a name with a newline used to panic GPUI's text shaping when its row
/// was drawn. The tall window draws every row; `alt-down` forces a frame.
#[gpui::test]
fn newline_in_a_name_does_not_panic_the_list(cx: &mut TestAppContext) {
    use crate::fuzz::Step;
    let tree = fuzz_tree("fuzz-newline-name", 40);
    let edge = EdgeTree::single(crow_fs::testing::EdgeKind::Newline);
    let found = replay_with(
        cx,
        &tree,
        0,
        &edge,
        1,
        false,
        crate::script![Step::Resize(1280.0, 1000.0), Step::Keys("alt-down".into())],
    );
    assert!(found.is_empty(), "{found:?}");
}

/// H4 #6 follow-up: the delete sheet names the file, and the bar draws operation
/// errors that name it. Both used to reach GPUI's text shaping raw.
#[gpui::test]
fn newline_in_a_name_does_not_panic_the_sheet_or_the_bar(cx: &mut TestAppContext) {
    let root = fixture("newline-sheet-bar");
    std::fs::write(root.join("line\nbreak.txt"), "x").expect("write");
    let (view, mut cx) = open(cx, &root);
    // Rows: alpha, beta, line\nbreak.txt, notes.txt.
    cx.simulate_keystrokes("down down cmd-alt-backspace");
    settle(&view, &mut cx);
    assert!(sheet_open(&view, &cx), "the delete sheet is open");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_keystrokes("escape");
    settle(&view, &mut cx);
    cx.update(|_, cx| crate::operations::message("“line\nbreak.txt” no longer exists", true, cx));
    settle(&view, &mut cx);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let message = cx.update(|_, cx| {
        cx.global::<crate::operations::Operations>().message.as_ref().map(|(text, _)| text.to_string())
    });
    assert_eq!(message.as_deref(), Some("“line break.txt” no longer exists"));
}

/// H4 #6 follow-up: a folder that fails to open names itself in the bar's error.
#[gpui::test]
fn newline_in_a_folder_error_does_not_panic_the_bar(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;

    let root = fixture("newline-folder-error");
    let unreadable = root.join("no\nread");
    std::fs::create_dir(&unreadable).expect("mkdir");
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    let (view, mut cx) = open(cx, &root);
    // Rows: alpha, beta, no\nread, notes.txt.
    cx.simulate_keystrokes("down down cmd-down");
    settle(&view, &mut cx);
    let error = bar_state(&view, &mut cx).error.map(|error| error.text.to_string());
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o755)).ok();
    let error = error.expect("the bar shows the load error");
    assert!(error.contains("no read"), "{error:?}");
    assert!(!error.contains('\n'), "{error:?}");
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// H4 #6 follow-up: the activity sheet draws a rename's old and new names.
#[gpui::test]
fn newline_in_a_rename_label_does_not_panic_the_activity(cx: &mut TestAppContext) {
    let root = fixture("newline-activity");
    std::fs::write(root.join("line\nbreak.txt"), "x").expect("write");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("down down enter");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("renamed.txt");
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    assert!(!root.join("line\nbreak.txt").exists(), "the rename applied");
    cx.simulate_keystrokes("alt-cmd-l");
    settle(&view, &mut cx);
    assert!(view.read_with(&cx, |w, _| w.show_activity), "the activity sheet is open");
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui::test]
fn fuzz_smoke(cx: &mut TestAppContext) {
    let tree = fuzz_tree("fuzz-smoke", 40);
    let options = FuzzOptions::new(40, EdgeTree::default(), false);
    // 30 steps per seed on a 40-file tree keeps the normal suite near the 10 s
    // budget; the long run uses 200 files and 3000 steps.
    for seed in [1, 2, 3] {
        if let Err(failure) = fuzz_run(seed, 30, &tree, &options, cx) {
            panic!("{}\n{}", failure.print(), failure.finding_row());
        }
    }
}

/// I2b: the Settings window as a second surface, driven by its own keymap and
/// control selectors. Two seeds keep the normal suite's time budget.
#[gpui::test]
fn fuzz_settings_smoke(cx: &mut TestAppContext) {
    for seed in [1, 2] {
        if let Err(failure) = settings_run(seed, 30, cx) {
            panic!("{}\n{}", failure.print(), failure.finding_row());
        }
    }
}

/// Q1: every small edge kind, including unreadable children and symlink loops;
/// `Wide` and `HugeFile` stay in `fuzz_long`.
#[gpui::test]
fn fuzz_smoke_edge_trees(cx: &mut TestAppContext) {
    let edge_tree = fuzz_tree("fuzz-smoke-edge", 40);
    let options = FuzzOptions::new(40, EdgeTree::small(), false);
    if let Err(failure) = fuzz_run(4, 30, &edge_tree, &options, cx) {
        panic!("{}\n{}", failure.print(), failure.finding_row());
    }
    let _ = crow_fs::testing::remove_tree(&edge_tree);
}

#[gpui::test]
#[ignore]
fn fuzz_long(cx: &mut TestAppContext) {
    let tree = fuzz_tree("fuzz-long", 200);
    let minutes = std::env::var("CROW_FUZZ_MINUTES").ok().and_then(|value| value.parse::<u64>().ok()).unwrap_or(1);
    let steps = std::env::var("CROW_FUZZ_STEPS").ok().and_then(|value| value.parse::<usize>().ok()).unwrap_or(3000);
    let edge = std::env::var("CROW_FUZZ_TREE")
        .ok()
        .map(|spec| EdgeTree::parse(&spec).unwrap_or_else(|error| panic!("CROW_FUZZ_TREE: {error}")))
        .unwrap_or_default();
    let churn_env = std::env::var("CROW_FUZZ_CHURN").ok().map(|value| value != "0");
    let explicit = std::env::var("CROW_FUZZ_SEED").ok().and_then(|value| value.parse::<u64>().ok());
    let run = |seed: u64, churn: bool, cx: &mut TestAppContext| {
        println!("CROW_FUZZ_SEED={seed} CROW_FUZZ_TREE={} CROW_FUZZ_CHURN={}", edge.spec(), if churn { 1 } else { 0 });
        let options = FuzzOptions::new(200, edge.clone(), churn);
        if let Err(failure) = fuzz_run(seed, steps, &tree, &options, cx) {
            println!("{}", failure.print());
            println!("{}", failure.finding_row());
            panic!("fuzz seed {seed} failed: {}", failure.violation.check);
        }
    };
    if let Some(seed) = explicit {
        run(seed, churn_env.unwrap_or(false), cx);
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(minutes * 60);
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |elapsed| elapsed.as_nanos() as u64);
    let mut index = 0;
    while Instant::now() < deadline {
        let churn = churn_env.unwrap_or(index % 2 == 1);
        run(seed, churn, cx);
        index += 1;
        seed = seed.wrapping_add(1);
    }
}

#[gpui::test]
fn filter_enter_acts_on_the_highlighted_row(cx: &mut TestAppContext) {
    let root = fixture("filter-enter-cursor");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("/");
    cx.simulate_input("a");
    settle(&view, &mut cx);

    // Down in the field moves the cursor with the highlight, so Return has a target.
    cx.simulate_keystrokes("down");
    let cursor = pane(&view, &cx).read_with(&cx, |p, cx| p.cursor_entry(cx).map(|entry| entry.name.clone()));
    assert_eq!(cursor.as_deref(), Some("beta"), "the cursor follows the highlight");
    assert_eq!(selected(&view, &cx).as_deref(), Some("beta"));

    // Return moves focus into the matches without moving the cursor.
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    let list_focus = pane(&view, &cx).read_with(&cx, |p, _| p.focus_handle().clone());
    assert!(cx.update(|window, _| list_focus.is_focused(window)), "Return moves focus into the matches");
    let cursor = pane(&view, &cx).read_with(&cx, |p, cx| p.cursor_entry(cx).map(|entry| entry.name.clone()));
    assert_eq!(cursor.as_deref(), Some("beta"), "focusing the list keeps the highlighted row");

    // Return in the list renames the cursor row (Crow's Enter is Rename; cmd-down opens).
    cx.simulate_keystrokes("enter");
    settle(&view, &mut cx);
    let renaming = pane(&view, &cx).read_with(&cx, |p, _| p.renaming_path());
    assert_eq!(renaming, Some(root.join("beta")), "Return acts on the cursor row");
}

/// H4 #12: a watcher batch used to prune the selection by hashing every row
/// (100k on the 100k fixture), so each batch took ~180 ms on the main thread.
/// Only the paths the batch removed can leave the selection now.
#[gpui::test]
fn a_watcher_batch_prunes_only_the_paths_it_removed(cx: &mut TestAppContext) {
    let root = fixture("fuzz-apply-batch");
    let folder = cx
        .update(|cx| cx.new(|cx| crate::folder::Folder::new(root.clone(), false, crow_fs::Sort::default(), false, cx)));
    let entries = |skip: Option<usize>| -> Vec<crow_fs::Entry> {
        (0..100_000)
            .filter(|i| Some(*i) != skip)
            .map(|i| crow_fs::Entry {
                name: format!("file-{i:06}.txt"),
                path: root.join(format!("file-{i:06}.txt")),
                is_dir: false,
                hidden: false,
                size: 0,
                modified: None,
                created: None,
                added: None,
            })
            .collect()
    };
    let removed = root.join("file-000000.txt");
    let stats = vec![(removed.clone(), None)];
    // Apply the listing without row 0 to a fully selected 100k listing, as a
    // watcher batch or as a full reload, and time only the apply: building the
    // entries and dropping the old listing stay outside the stopwatch. CI runs a
    // debug build on a shared runner, so the batch is compared with the full
    // prune on the same machine rather than with a fixed budget (the bench owns that).
    let apply = |stats: Option<&[(std::path::PathBuf, Option<crow_fs::Entry>)]>| {
        cx.update(|cx| folder.update(cx, |folder, cx| folder.replace_entries_for_test(entries(None), Some(&[]), cx)));
        cx.update(|cx| folder.update(cx, |folder, cx| folder.select_all(cx)));
        let old = cx.update(|cx| folder.read(cx).entries().clone());
        let next = entries(Some(0));
        let started = std::time::Instant::now();
        cx.update(|cx| folder.update(cx, |folder, cx| folder.replace_entries_for_test(next, stats, cx)));
        let elapsed = started.elapsed();
        drop(old);
        elapsed
    };
    let full = (0..3).map(|_| apply(None)).min().unwrap();
    let batch = (0..3).map(|_| apply(Some(&stats))).min().unwrap();
    let selection = cx.update(|cx| folder.read(cx).selection_paths());
    assert_eq!(selection.len(), 99_999, "only the removed path left the selection");
    assert!(!selection.contains(&removed));
    assert!(batch * 10 < full, "the batch apply took {batch:?}, a full prune {full:?}");
}

/// H4 batch 6: the bottom bar asks for the selection's bytes every frame, so the
/// totals are kept current by each selection change instead of rescanning a 100k
/// listing whenever the selection generation bumps.
#[gpui::test]
fn selection_totals_follow_every_selection_change(cx: &mut TestAppContext) {
    let root = fixture("selection-totals");
    cx.update(crate::ui::init_for_tests);
    let folder = cx
        .update(|cx| cx.new(|cx| crate::folder::Folder::new(root.clone(), false, crow_fs::Sort::default(), false, cx)));
    let entry = |name: &str, size: u64, is_dir: bool| crow_fs::Entry {
        name: name.to_owned(),
        path: root.join(name),
        is_dir,
        hidden: false,
        size,
        modified: None,
        created: None,
        added: None,
    };
    let entries =
        vec![entry("a.txt", 10, false), entry("b.txt", 20, false), entry("dir", 0, true), entry("c.txt", 30, false)];
    let totals = |cx: &mut TestAppContext| cx.update(|cx| folder.read(cx).selected_bytes());
    let apply = |cx: &mut TestAppContext,
                 entries: Vec<crow_fs::Entry>,
                 stats: Option<&[(std::path::PathBuf, Option<crow_fs::Entry>)]>| {
        cx.update(|cx| folder.update(cx, |folder, cx| folder.replace_entries_for_test(entries, stats, cx)));
    };
    apply(cx, entries.clone(), None);

    cx.update(|cx| folder.update(cx, |folder, cx| folder.select_only(0, cx)));
    assert_eq!(totals(cx), (10, 0), "a plain click selects one file");
    cx.update(|cx| folder.update(cx, |folder, cx| folder.toggle(1, cx)));
    assert_eq!(totals(cx), (30, 0), "cmd-click adds a file");
    cx.update(|cx| folder.update(cx, |folder, cx| folder.toggle(2, cx)));
    assert_eq!(totals(cx), (30, 1), "cmd-click adds a folder, which adds no bytes");
    cx.update(|cx| folder.update(cx, |folder, cx| folder.toggle(1, cx)));
    assert_eq!(totals(cx), (10, 1), "cmd-click removes a file");
    cx.update(|cx| folder.update(cx, |folder, cx| folder.clear_selection(cx)));
    cx.update(|cx| folder.update(cx, |folder, cx| folder.select_only(0, cx)));
    cx.update(|cx| folder.update(cx, |folder, cx| folder.extend_to(3, cx)));
    assert_eq!(totals(cx), (60, 1), "shift-click sums the range it selects");
    cx.update(|cx| folder.update(cx, |folder, cx| folder.select_all(cx)));
    assert_eq!(totals(cx), (60, 1), "select-all sums every row once");
    cx.update(|cx| folder.update(cx, |folder, cx| folder.clear_selection(cx)));
    assert_eq!(totals(cx), (0, 0), "clearing drops the totals");

    // A watcher batch that removes a selected file subtracts it.
    cx.update(|cx| folder.update(cx, |folder, cx| folder.select_all(cx)));
    let removed = vec![(root.join("a.txt"), None)];
    apply(cx, entries[1..].to_vec(), Some(&removed));
    assert_eq!(totals(cx), (50, 1), "a removed file leaves the totals");

    // A full apply that drops a selected row prunes and sums in one pass.
    cx.update(|cx| folder.update(cx, |folder, cx| folder.select_all(cx)));
    apply(cx, entries[1..3].to_vec(), None);
    assert_eq!(totals(cx), (20, 1), "a full apply sums what the selection kept");

    // A watcher batch that changes a selected file's size updates the totals.
    apply(cx, entries.clone(), None);
    cx.update(|cx| folder.update(cx, |folder, cx| folder.select_only(0, cx)));
    let mut bigger = entries.clone();
    bigger[0].size = 99;
    let changed = vec![(root.join("a.txt"), Some(bigger[0].clone()))];
    apply(cx, bigger, Some(&changed));
    assert_eq!(totals(cx), (99, 0), "a changed file size updates the totals");
    // Deselecting the changed file must subtract its new size once, not the old one.
    cx.update(|cx| folder.update(cx, |folder, cx| folder.toggle(1, cx)));
    assert_eq!(totals(cx), (119, 0), "cmd-click adds a file after the change");
    cx.update(|cx| folder.update(cx, |folder, cx| folder.toggle(0, cx)));
    assert_eq!(totals(cx), (20, 0), "deselecting the changed file leaves the rest");
}

/// H4 batch 6: a watcher batch that removes the selected row drops it from the
/// selection before the new rows are built; the cursor must not keep naming it
/// (the fuzzer's cursor check: the cursor row must be selected).
#[gpui::test]
fn a_removed_selected_row_does_not_keep_the_cursor(cx: &mut TestAppContext) {
    let root = fixture("removed-cursor");
    cx.update(crate::ui::init_for_tests);
    let folder = cx
        .update(|cx| cx.new(|cx| crate::folder::Folder::new(root.clone(), false, crow_fs::Sort::default(), false, cx)));
    let entry = |name: &str| crow_fs::Entry {
        name: name.to_owned(),
        path: root.join(name),
        is_dir: false,
        hidden: false,
        size: 1,
        modified: None,
        created: None,
        added: None,
    };
    let entries = vec![entry("a.txt"), entry("b.txt")];
    cx.update(|cx| folder.update(cx, |folder, cx| folder.replace_entries_for_test(entries.clone(), None, cx)));
    cx.update(|cx| folder.update(cx, |folder, cx| folder.select_only(0, cx)));
    // a.txt leaves, as a rename's watcher batch reports it.
    let removed = vec![(root.join("a.txt"), None)];
    cx.update(|cx| {
        folder.update(cx, |folder, cx| folder.replace_entries_for_test(entries[1..].to_vec(), Some(&removed), cx))
    });
    let (cursor, selected) = cx.update(|cx| {
        let folder = folder.read(cx);
        (folder.cursor_entry().map(|entry| entry.path.clone()), folder.selection_paths())
    });
    assert_ne!(cursor, Some(root.join("a.txt")), "the cursor left the removed row");
    assert!(cursor.as_ref().is_none_or(|path| selected.contains(path)), "the cursor row is selected");
    assert_eq!(cursor, Some(root.join("b.txt")), "the row now at the removed row's index is selected");
    assert_eq!(selected, vec![root.join("b.txt")], "the next row is the whole selection");
}

/// Build a folder over synthetic entries, without touching the disk (the watcher
/// still watches `root`; the tests apply their own listings).
fn synthetic_folder(
    cx: &mut TestAppContext,
    root: &std::path::Path,
    names: &[&str],
) -> (Entity<crate::folder::Folder>, Vec<crow_fs::Entry>) {
    cx.update(crate::ui::init_for_tests);
    let folder = cx.update(|cx| {
        cx.new(|cx| crate::folder::Folder::new(root.to_path_buf(), false, crow_fs::Sort::default(), false, cx))
    });
    let entries: Vec<crow_fs::Entry> = names
        .iter()
        .map(|name| crow_fs::Entry {
            name: (*name).to_owned(),
            path: root.join(name),
            is_dir: false,
            hidden: false,
            size: 1,
            modified: None,
            created: None,
            added: None,
        })
        .collect();
    cx.update(|cx| folder.update(cx, |folder, cx| folder.replace_entries_for_test(entries.clone(), None, cx)));
    (folder, entries)
}

fn cursor_and_selection(
    cx: &mut TestAppContext,
    folder: &Entity<crate::folder::Folder>,
) -> (Option<PathBuf>, Vec<PathBuf>) {
    cx.update(|cx| {
        let folder = folder.read(cx);
        (folder.cursor_entry().map(|entry| entry.path.clone()), folder.selection_paths())
    })
}

#[gpui::test]
fn a_watcher_removal_of_the_last_row_selects_the_previous_row(cx: &mut TestAppContext) {
    let root = fixture("removed-last-cursor");
    let (folder, entries) = synthetic_folder(cx, &root, &["a.txt", "b.txt", "c.txt"]);
    cx.update(|cx| folder.update(cx, |folder, cx| folder.select_only(2, cx)));
    let removed = vec![(root.join("c.txt"), None)];
    cx.update(|cx| {
        folder.update(cx, |folder, cx| folder.replace_entries_for_test(entries[..2].to_vec(), Some(&removed), cx))
    });
    let (cursor, selected) = cursor_and_selection(cx, &folder);
    assert_eq!(cursor, Some(root.join("b.txt")), "the new last row is selected");
    assert_eq!(selected, vec![root.join("b.txt")]);
}

#[gpui::test]
fn a_watcher_removal_of_another_row_does_not_move_the_cursor(cx: &mut TestAppContext) {
    let root = fixture("removed-other-cursor");
    let (folder, entries) = synthetic_folder(cx, &root, &["a.txt", "b.txt", "c.txt"]);
    cx.update(|cx| folder.update(cx, |folder, cx| folder.select_only(0, cx)));
    let removed = vec![(root.join("b.txt"), None)];
    cx.update(|cx| {
        folder.update(cx, |folder, cx| {
            folder.replace_entries_for_test(vec![entries[0].clone(), entries[2].clone()], Some(&removed), cx)
        })
    });
    let (cursor, selected) = cursor_and_selection(cx, &folder);
    assert_eq!(cursor, Some(root.join("a.txt")), "a file vanishing elsewhere leaves the cursor");
    assert_eq!(selected, vec![root.join("a.txt")]);
}

#[gpui::test]
fn an_empty_folder_after_a_removal_has_no_selection(cx: &mut TestAppContext) {
    let root = fixture("removed-empty-cursor");
    let (folder, _entries) = synthetic_folder(cx, &root, &["a.txt"]);
    cx.update(|cx| folder.update(cx, |folder, cx| folder.select_only(0, cx)));
    let removed = vec![(root.join("a.txt"), None)];
    cx.update(|cx| folder.update(cx, |folder, cx| folder.replace_entries_for_test(Vec::new(), Some(&removed), cx)));
    let (cursor, selected) = cursor_and_selection(cx, &folder);
    assert_eq!(cursor, None, "nothing is left to select");
    assert!(selected.is_empty());
}

#[gpui::test]
fn a_watcher_removal_in_a_filtered_listing_selects_the_next_visible_row(cx: &mut TestAppContext) {
    let root = fixture("removed-filtered-cursor");
    let (folder, entries) = synthetic_folder(cx, &root, &["apple.txt", "banana.txt", "cherry.txt", "date.txt"]);
    cx.update(|cx| folder.update(cx, |folder, cx| folder.set_filter(Some("e".into()), cx)));
    // Filtered rows: apple.txt, cherry.txt, date.txt (banana.txt has no e). Select cherry.txt.
    let shown: Vec<String> = cx.update(|cx| {
        let folder = folder.read(cx);
        (0..folder.row_count()).map(|ix| folder.row(ix).expect("row").0.name.clone()).collect()
    });
    assert_eq!(shown, ["apple.txt", "cherry.txt", "date.txt"], "the filter shows three rows");
    cx.update(|cx| folder.update(cx, |folder, cx| folder.select_only(1, cx)));
    let removed = vec![(root.join("cherry.txt"), None)];
    cx.update(|cx| {
        folder.update(cx, |folder, cx| {
            folder.replace_entries_for_test(
                vec![entries[0].clone(), entries[1].clone(), entries[3].clone()],
                Some(&removed),
                cx,
            )
        })
    });
    let (cursor, selected) = cursor_and_selection(cx, &folder);
    assert_eq!(cursor, Some(root.join("date.txt")), "the next row the filter shows");
    assert_eq!(selected, vec![root.join("date.txt")]);
}

/// A child row vanishing from an inline folder leaves the parent row's cursor alone.
#[gpui::test]
fn a_child_removal_keeps_the_parent_cursor(cx: &mut TestAppContext) {
    let root = fixture("child-removal-parent");
    let (view, mut cx) = open(cx, &root);
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "alpha is the cursor");
    cx.simulate_keystrokes("right");
    settle(&view, &mut cx);
    assert!(names(&view, &cx).contains(&"inner".to_owned()), "alpha is open");
    let folder = pane(&view, &cx).read_with(&cx, |pane, _| pane.test_folder());
    let alpha = root.join("alpha");
    let child = folder.read_with(&cx, |folder, _| folder.child_for_test(&alpha)).expect("alpha is open");
    let child_entries = child.read_with(&cx, |folder, _| folder.entries().as_ref().clone());
    let removed = vec![(alpha.join("inner"), None)];
    let kept: Vec<Entry> = child_entries.iter().filter(|entry| entry.name != "inner").cloned().collect();
    child.update(&mut cx, |folder, cx| folder.replace_entries_for_test(kept, Some(&removed), cx));
    settle(&view, &mut cx);
    assert!(!names(&view, &cx).contains(&"inner".to_owned()), "the child row left");
    assert_eq!(selected(&view, &cx).as_deref(), Some("alpha"), "the parent row stays the cursor");
}

/// H4 #22: a staged load shows the rows it has read so far. A partial must not
/// prune the selection or close an inline folder for a row it has not reached;
/// the full listing settles both when it lands.
#[gpui::test]
fn a_partial_listing_keeps_the_selection_and_open_folders(cx: &mut TestAppContext) {
    let root = fixture("partial-apply");
    let (view, mut cx) = open(cx, &root);
    let folder = pane(&view, &cx).read_with(&cx, |pane, _| pane.test_folder());
    let alpha = root.join("alpha");
    let notes = root.join("notes.txt");

    folder.update(&mut cx, |folder, cx| {
        let ix = folder.entries().iter().position(|entry| entry.path == notes).expect("notes.txt");
        folder.select_only(ix, cx);
    });
    folder.update(&mut cx, |folder, cx| folder.expand(alpha.clone(), cx));
    settle(&view, &mut cx);
    let all = folder.read_with(&cx, |folder, _| folder.entries().as_ref().clone());
    let partial: Vec<Entry> =
        all.iter().filter(|entry| entry.name != "alpha" && entry.name != "notes.txt").cloned().collect();
    folder.update(&mut cx, |folder, cx| folder.apply_partial_for_test(partial, cx));

    let (selection, expanded) =
        folder.read_with(&cx, |folder, cx| (folder.selection_paths(), folder.is_expanded(&alpha, cx)));
    assert!(selection.contains(&notes), "a partial keeps the selection: {selection:?}");
    assert!(expanded, "a partial keeps an inline folder open");

    // The full listing restores the rows, the cursor and alpha's child rows.
    folder.update(&mut cx, |folder, cx| folder.replace_entries_for_test(all, None, cx));
    let rows = rows(&view, &cx);
    assert!(rows.iter().any(|(name, _)| name == "notes.txt"), "{rows:?}");
    assert!(rows.iter().any(|(name, _)| name == "inner"), "alpha's child rows return: {rows:?}");
    assert_eq!(selected(&view, &cx).as_deref(), Some("notes.txt"), "the cursor follows the selection");
}

/// H4 #22: collapsing an inline folder frees its listing before the child
/// entity drops. A 100k listing dropped on the main thread is a visible hitch.
#[gpui::test]
fn collapsing_an_inline_folder_releases_its_listing(cx: &mut TestAppContext) {
    let root = fixture("collapse-release");
    let (view, mut cx) = open(cx, &root);
    let folder = pane(&view, &cx).read_with(&cx, |pane, _| pane.test_folder());
    let alpha = root.join("alpha");
    folder.update(&mut cx, |folder, cx| folder.expand(alpha.clone(), cx));
    settle(&view, &mut cx);
    let child = folder.read_with(&cx, |folder, _| folder.child_for_test(&alpha)).expect("alpha is open");
    assert!(!child.read_with(&cx, |folder, _| folder.entries().is_empty()), "the child listed its folder");

    folder.update(&mut cx, |folder, cx| folder.collapse(&alpha, cx));
    assert!(
        child.read_with(&cx, |folder, _| folder.entries().is_empty()),
        "collapse releases the child's listing before the entity drops"
    );
}

/// H4 #22/#26: a 100k duplicate or paste sets `select_when_present` to 100k
/// paths. The per-path scan of the listing was O(n²) and held the main thread
/// for 30 s on one apply (the seed-505 live run's HANG at step 9112).
#[gpui::test]
fn a_large_select_when_present_applies_quickly(cx: &mut TestAppContext) {
    let root = fixture("select-when-present");
    let folder = cx
        .update(|cx| cx.new(|cx| crate::folder::Folder::new(root.clone(), false, crow_fs::Sort::default(), false, cx)));
    let entries: Vec<Entry> = (0..100_000)
        .map(|i| Entry {
            name: format!("file-{i:06}.txt"),
            path: root.join(format!("file-{i:06}.txt")),
            is_dir: false,
            hidden: false,
            size: 0,
            modified: None,
            created: None,
            added: None,
        })
        .collect();
    let paths: Vec<PathBuf> = entries.iter().map(|entry| entry.path.clone()).collect();
    cx.update(|cx| {
        folder.update(cx, |folder, cx| {
            folder.replace_entries_for_test(entries.clone(), Some(&[]), cx);
            folder.select_when_present_for_test(paths, crow_core::motion::Effective::Off);
        });
    });
    let started = Instant::now();
    cx.update(|cx| folder.update(cx, |folder, cx| folder.replace_entries_for_test(entries, Some(&[]), cx)));
    let elapsed = started.elapsed();
    let selection = cx.update(|cx| folder.read(cx).selection_paths());
    assert_eq!(selection.len(), 100_000, "every result is selected");
    assert!(elapsed < Duration::from_secs(2), "the apply took {elapsed:?}");
}

/// H4 #20: the 100 ms watcher poll used to spawn a second batch while the first
/// was still stat'ing, and both applies could land in one frame (a 152 ms step
/// on the 100k fixture). A batch now waits for the one in flight.
#[gpui::test]
fn watcher_batches_apply_one_at_a_time(cx: &mut TestAppContext) {
    let root = fixture("fuzz-apply-coalesce");
    std::fs::write(root.join("a.txt"), "a").expect("write a");
    std::fs::write(root.join("b.txt"), "b").expect("write b");
    let folder = cx
        .update(|cx| cx.new(|cx| crate::folder::Folder::new(root.clone(), false, crow_fs::Sort::default(), false, cx)));
    cx.update(|cx| folder.update(cx, |folder, _| folder.set_apply_delay(Duration::from_millis(300))));
    cx.update(|cx| folder.update(cx, |folder, _| folder.queue_changes(vec![root.join("a.txt")])));
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| folder.read(cx).apply_batches()), 1, "the first batch is in flight");
    cx.update(|cx| folder.update(cx, |folder, _| folder.queue_changes(vec![root.join("b.txt")])));
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| folder.read(cx).apply_batches()), 1, "a second batch waits for the first");
    // The stat and merge run on the background executor; give them real time too.
    let mut batches = 0;
    for _ in 0..100 {
        cx.executor().advance_clock(Duration::from_millis(400));
        cx.run_until_parked();
        batches = cx.update(|cx| folder.read(cx).apply_batches());
        if batches == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(batches, 2, "the queued paths run as the next batch");
    // The rows are flattened on the background executor; wait for them to land.
    let mut names = Vec::new();
    for _ in 0..100 {
        cx.run_until_parked();
        names = cx.update(|cx| folder.read(cx).names());
        if names.contains(&"a.txt".to_owned()) && names.contains(&"b.txt".to_owned()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(names.contains(&"a.txt".to_owned()) && names.contains(&"b.txt".to_owned()), "{names:?}");
}

/// H4 #15: FSEvents can coalesce away the delete that follows a rename, leaving
/// a row the disk no longer has. The quiet reconcile re-stats the paths the
/// batch added; a delete inside that window drops the phantom.
#[gpui::test]
fn a_delete_lost_after_a_rename_is_reconciled(cx: &mut TestAppContext) {
    let root = fixture("fuzz-reconcile");
    let (view, mut cx) = open(cx, &root);
    let folder = pane(&view, &cx).read_with(&cx, |pane, _| pane.test_folder());
    let renamed = root.join("churn-renamed-000525");
    std::fs::write(&renamed, "x").expect("write the rename target");
    folder.update(&mut cx, |folder, _| folder.queue_changes(vec![renamed.clone()]));
    // Let the 100 ms poll apply the batch, but not the 300 ms reconcile.
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    assert!(rows(&view, &cx).iter().any(|(name, _)| name == "churn-renamed-000525"), "the rename row was added");
    // The delete's watcher event is lost; only the reconcile can see it.
    std::fs::remove_file(&renamed).expect("remove the rename target");
    cx.executor().advance_clock(Duration::from_millis(350));
    cx.run_until_parked();
    assert!(
        !rows(&view, &cx).iter().any(|(name, _)| name == "churn-renamed-000525"),
        "the quiet reconcile dropped the phantom row"
    );
}

#[gpui::test]
fn interactive_elements_have_roles_and_labels(cx: &mut TestAppContext) {
    let root = fixture("a11y-interactive");
    let (_view, mut cx) = open(cx, &root);
    let elements = cx.update(|window, _| window.debug_interactive_elements().expect("recorded").to_vec());
    let missing: Vec<_> = elements
        .iter()
        .filter(|element| element.role.is_none() || element.label.as_ref().is_none_or(|label| label.trim().is_empty()))
        .map(|element| format!("{:?} role={:?} label={:?}", element.selector, element.role, element.label))
        .collect();
    assert!(missing.is_empty(), "{missing:#?}");
    let back = elements.iter().find(|element| element.selector.as_deref() == Some("back")).expect("back button");
    assert_eq!(back.role, Some(Role::Button));
    assert_eq!(back.label.as_deref(), Some("Back"));
    assert_eq!(back.disabled, Some(true), "a fresh window has nowhere to go back to");
}

#[gpui::test]
fn path_edit_keeps_the_focus_invariant(cx: &mut TestAppContext) {
    let root = fixture("path-edit-invariant");
    let (view, mut cx) = open(cx, &root);
    cx.simulate_keystrokes("cmd-l");
    settle(&view, &mut cx);
    assert!(violations(&view, &mut cx).is_empty(), "the path field is a focus surface");
    assert!(crate::fuzz::state(&view, &mut cx).text_focused, "the fuzzer types into the path field");

    // Search can stay open behind the field; the bar's field still owns focus.
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("cmd-f");
    settle(&view, &mut cx);
    cx.simulate_keystrokes("cmd-l");
    settle(&view, &mut cx);
    assert!(violations(&view, &mut cx).is_empty(), "the path field is a focus surface over search");
    assert!(crate::fuzz::state(&view, &mut cx).text_focused, "…and the fuzzer still types into it");
}

#[gpui::test]
fn column_view_announces_columns_once(cx: &mut TestAppContext) {
    let root = fixture("a11y-columns");
    let (view, mut cx) = open(cx, &root);
    pane(&view, &cx).update(&mut cx, |pane, cx| pane.set_mode(ViewMode::Columns, cx));
    settle(&view, &mut cx);
    let elements = cx.update(|window, _| window.debug_interactive_elements().expect("recorded").to_vec());
    let wrapper = elements.iter().find(|element| element.selector.as_deref() == Some("file-list")).expect("file list");
    assert_eq!(wrapper.role, Some(Role::Group), "the focusable wrapper is not a list");
    assert_eq!(wrapper.label.as_deref(), Some("Columns"), "it does not repeat the active column's label");
}
