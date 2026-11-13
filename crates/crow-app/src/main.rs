//! Crow, a Finder-style file explorer.
//!
//! ```text
//! crow [DIR]                       open a window on DIR (default: home)
//! crow --bench DIR                 auto-scroll DIR for 600 frames, print frame times, quit
//! crow --bench-filter QUERY DIR    the same, with the name filter applied first
//! crow --make-fixture DIR [N]      create N (default 100,000) empty files in DIR
//! crow --make-search-fixture DIR [N]  create N (default 200,000) nested text files
//! crow --bench-search DIR [QUERY]  time a name+contents walk, print first hit and total
//! ```

mod actions;
mod chrome;
mod extensions;
mod folder;
mod icons;
mod keys;
mod logging;
mod menus;
mod operations;
mod palette;
mod pane;
mod preview;
mod search;
mod sidebar;
mod style;
mod text_input;
mod ui;
mod window;

use std::path::PathBuf;
use std::process::ExitCode;

use gpui::App;

/// A nested tree for search benches: 32 groups x 16 subfolders, each file a few
/// lines with `needle` so both name and content searches do real work.
fn make_search_fixture(dir: &std::path::Path, count: usize) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let (groups, subs) = (32usize, 16usize);
    let per_folder = count.div_ceil(groups * subs);
    let mut made = 0;
    for group in 0..groups {
        for sub in 0..subs {
            let folder = dir.join(format!("group{group:02}")).join(format!("sub{sub:02}"));
            std::fs::create_dir_all(&folder)?;
            for i in 0..per_folder {
                if made >= count {
                    break;
                }
                let name = format!("file-{group:02}-{sub:02}-{i:05}.txt");
                std::fs::write(folder.join(name), format!("alpha beta gamma\nneedle in {group}/{sub}/{i}\n"))?;
                made += 1;
            }
        }
    }
    Ok(())
}

fn bench_search(dir: &std::path::Path, text: &str) -> Result<(), String> {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    let root = dir.canonicalize().map_err(|e| format!("{}: {e}", dir.display()))?;
    let rules = crow_search::RuleSet::from_config(&crow_core::Exclude::default(), &crow_core::home())?;
    let query = crow_search::Query {
        names: Some(text.to_owned()),
        contents: Some(text.to_owned()),
        show_hidden: false,
        rules,
        everything: false,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let started = std::time::Instant::now();
    let mut first_hit = None;
    let mut hits = 0;
    let summary = crow_search::search(
        &root,
        &query,
        |batch| {
            if first_hit.is_none() {
                first_hit = Some(started.elapsed());
            }
            hits += batch.len();
        },
        &cancel,
    );
    println!(
        "SEARCH first_hit={:.0}ms total={:.0}ms hits={hits} scanned={} skipped={}",
        first_hit.unwrap_or_default().as_secs_f64() * 1000.0,
        started.elapsed().as_secs_f64() * 1000.0,
        summary.scanned,
        summary.skipped.len()
    );
    Ok(())
}

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

    if args.first().map(String::as_str) == Some("--make-search-fixture") {
        let Some(dir) = args.get(1).map(PathBuf::from) else {
            eprintln!("usage: crow --make-search-fixture DIR [N]");
            return ExitCode::FAILURE;
        };
        let count = args.get(2).and_then(|n| n.parse().ok()).unwrap_or(200_000);
        return match make_search_fixture(&dir, count) {
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

    if args.first().map(String::as_str) == Some("--bench-search") {
        let Some(dir) = args.get(1).map(PathBuf::from) else {
            eprintln!("usage: crow --bench-search DIR [QUERY]");
            return ExitCode::FAILURE;
        };
        let query = args.get(2).map(String::as_str).unwrap_or("needle");
        return match bench_search(&dir, query) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}: {e}", dir.display());
                ExitCode::FAILURE
            }
        };
    }

    let bench_filter_at = args.iter().position(|a| a == "--bench-filter");
    let bench_filter = bench_filter_at.and_then(|i| args.get(i + 1).cloned());
    let bench = args.iter().any(|a| a == "--bench") || bench_filter.is_some();
    let start = args
        .iter()
        .enumerate()
        .find(|(i, arg)| !arg.starts_with("--") && Some(*i) != bench_filter_at.map(|at| at + 1))
        .map(|(_, arg)| PathBuf::from(arg))
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"));
    let start = match start.canonicalize() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("{}: {e}", start.display());
            return ExitCode::FAILURE;
        }
    };

    gpui_platform::application().with_assets(icons::Assets).run(move |cx: &mut App| {
        ui::init(cx);
        extensions::init(cx);
        operations::init(cx, std::sync::Arc::new(crow_macos::SystemTrash));
        cx.bind_keys(actions::finder_keymap());
        cx.bind_keys(actions::vim_keymap());
        cx.bind_keys(text_input::keymap());
        menus::init(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        window::open(start.clone(), bench, bench_filter.clone(), cx);
        cx.activate(true);
    });
    ExitCode::SUCCESS
}
