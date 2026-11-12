//! Crow, a Finder-style file explorer.
//!
//! ```text
//! crow [DIR]                       open a window on DIR (default: home)
//! crow --bench DIR                 auto-scroll DIR for 600 frames, print frame times, quit
//! crow --make-fixture DIR [N]      create N (default 100,000) empty files in DIR
//! ```

mod actions;
mod extensions;
mod folder;
mod logging;
mod pane;
mod preview;
mod style;
mod text_input;
mod ui;
mod window;

use std::path::PathBuf;
use std::process::ExitCode;

use gpui::{App, Menu, MenuItem};

use crate::actions::{NewTab, NewWindow, Quit};

fn main() -> ExitCode {
    let _log_guard = logging::init();
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.first().map(String::as_str) == Some("--make-fixture") {
        let Some(dir) = args.get(1).map(PathBuf::from) else {
            eprintln!("usage: crow --make-fixture DIR [N]");
            return ExitCode::FAILURE;
        };
        let count = args.get(2).and_then(|n| n.parse().ok()).unwrap_or(100_000);
        return match crow_fs::make_fixture(&dir, count) {
            Ok(()) => {
                println!("created {count} files in {}", dir.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{}: {e}", dir.display());
                ExitCode::FAILURE
            }
        };
    }

    let bench = args.iter().any(|a| a == "--bench");
    let start = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"));
    let start = match start.canonicalize() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("{}: {e}", start.display());
            return ExitCode::FAILURE;
        }
    };

    gpui_platform::application().run(move |cx: &mut App| {
        ui::init(cx);
        extensions::init(cx);
        cx.bind_keys(actions::finder_keymap());
        cx.bind_keys(text_input::keymap());
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &NewWindow, cx| {
            let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
            window::open(home, false, cx);
        });
        cx.set_menus([
            Menu { name: "Crow".into(), items: vec![MenuItem::action("Quit Crow", Quit)], disabled: false },
            Menu {
                name: "File".into(),
                items: vec![MenuItem::action("New Window", NewWindow), MenuItem::action("New Tab", NewTab)],
                disabled: false,
            },
        ]);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        window::open(start.clone(), bench, cx);
        cx.activate(true);
    });
    ExitCode::SUCCESS
}
