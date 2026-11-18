//! Crow, a Finder-style file explorer.
//!
//! ```text
//! crow [DIR]                       open a window on DIR (default: home)
//! crow --bench DIR                 auto-scroll DIR for 600 frames, print frame times, quit
//! crow --bench-filter QUERY DIR    the same, with the name filter applied first
//! crow --fuzz [DIR --dir-is-scratch] [--seed N] [--minutes M]  drive a real window
//!                                  with H1's generator and report slow frames (needs --features fuzz)
//! crow --make-fixture DIR [N]      create N (default 100,000) empty files in DIR
//! crow --make-search-fixture DIR [N]  create N (default 200,000) nested text files
//! crow --make-search-corpus DIR   create the deterministic search corpus (fixtures + named cases)
//! crow --bench-search DIR [QUERY] [--names|--contents]
//!                                  time a walk, print first hit, first name hit and totals
//! crow --search-trace DIR QUERY [--target PATH] [--names|--contents] [--hidden|--no-hidden] [--everything]
//!                                  run the real engine and print each batch, the totals and one verdict
//! crow --motion-lab                a fake folder to try motion modes and feedback sounds
//! ```

// The `fuzz` feature builds the generator and invariants into the binary for
// H3's `crow --fuzz`; until that entry point exists nothing outside tests calls them.
#![cfg_attr(all(feature = "fuzz", not(test)), allow(dead_code))]

mod actions;
mod bottom_bar;
mod chrome;
mod context_menu;
mod controls;
mod effects;
mod extensions;
mod feedback;
mod folder;
mod folder_sizes;
#[cfg(any(test, feature = "fuzz"))]
mod fuzz;
#[cfg(feature = "fuzz")]
mod fuzz_live;
mod icons;
mod info_pane;
#[cfg(test)]
mod io_guard;
mod keys;
mod logging;
mod menus;
mod motion_lab;
mod onboarding;
mod operations;
mod palette;
mod pane;
mod preview;
mod search;
mod settings_window;
mod sidebar;
mod style;
mod syntax;
mod text_input;
mod theme_choice;
mod tips;
mod trash;
mod ui;
mod usage;
mod window;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{App, AppContext};

/// A nested tree for search benches: 32 groups x 16 subfolders, each file a few
/// lines with `needle` so both name and content searches do real work.
fn make_search_fixture(dir: &std::path::Path, count: usize) -> std::io::Result<()> {
    // io-ok: CLI fixture, before any window opens
    std::fs::create_dir_all(dir)?;
    let (groups, subs) = (32usize, 16usize);
    let per_folder = count.div_ceil(groups * subs);
    let mut made = 0;
    for group in 0..groups {
        for sub in 0..subs {
            let folder = dir.join(format!("group{group:02}")).join(format!("sub{sub:02}"));
            // io-ok: CLI fixture, before any window opens
            std::fs::create_dir_all(&folder)?;
            for i in 0..per_folder {
                if made >= count {
                    break;
                }
                let name = format!("file-{group:02}-{sub:02}-{i:05}.txt");
                // io-ok: CLI fixture, before any window opens
                std::fs::write(folder.join(name), format!("alpha beta gamma\nneedle in {group}/{sub}/{i}\n"))?;
                made += 1;
            }
        }
    }
    Ok(())
}

/// Time a walk through the real engine. This is an engine bench: it does not
/// apply the app's result cap, so it measures the walk, the matcher and the
/// content reads rather than the panel's budgets.
fn bench_search(args: &[String]) -> Result<(), String> {
    use std::sync::Arc;

    let Some(dir) = args.first() else {
        return Err("usage: crow --bench-search DIR [QUERY] [--names|--contents]".to_owned());
    };
    let mut text = "needle".to_owned();
    let mut query_set = false;
    let mut names = true;
    let mut contents = true;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--names" => {
                names = true;
                contents = false;
                index += 1;
            }
            "--contents" => {
                names = false;
                contents = true;
                index += 1;
            }
            other if other.starts_with("--") => return Err(format!("unknown argument {other}")),
            other => {
                if query_set {
                    return Err(format!("unexpected argument {other}"));
                }
                text = other.to_owned();
                query_set = true;
                index += 1;
            }
        }
    }
    let dir = std::path::Path::new(dir);
    // io-ok: CLI bench, before any window opens
    let root = dir.canonicalize().map_err(|e| format!("{}: {e}", dir.display()))?;
    let rules = crow_search::RuleSet::from_config(&crow_core::Exclude::default(), &crow_core::home())?;
    let query = crow_search::Query {
        names: names.then(|| text.clone()),
        contents: contents.then(|| text.clone()),
        enter_hidden_folders: false,
        include_hidden: true,
        rules: Arc::new(rules),
        everything: false,
    };
    let control = crow_search::Control::new();
    let started = std::time::Instant::now();
    let mut first_hit = None;
    let mut first_name_hit = None;
    let mut hits = 0;
    let summary = crow_search::search(
        &root,
        &query,
        |batch| {
            let elapsed = started.elapsed();
            if first_hit.is_none() {
                first_hit = Some(elapsed);
            }
            if first_name_hit.is_none() && batch.iter().any(|hit| matches!(hit.kind, crow_search::HitKind::Name { .. }))
            {
                first_name_hit = Some(elapsed);
            }
            hits += batch.len();
        },
        &control,
    );
    let ms = |time: Option<std::time::Duration>| {
        time.map_or_else(|| "none".to_owned(), |time| format!("{:.1}ms", time.as_secs_f64() * 1000.0))
    };
    println!(
        "SEARCH first_hit={} first_name_hit={} total={:.1}ms hits={hits} scanned={} skipped={} contents_stopped={}",
        ms(first_hit),
        ms(first_name_hit),
        started.elapsed().as_secs_f64() * 1000.0,
        summary.scanned,
        summary.skipped.len(),
        summary.contents_stopped
    );
    Ok(())
}

struct TraceOptions {
    dir: PathBuf,
    query: String,
    target: Option<PathBuf>,
    names: bool,
    contents: bool,
    show_hidden: bool,
    search_includes_hidden: bool,
    everything: bool,
}

fn parse_search_trace(args: &[String]) -> Result<TraceOptions, String> {
    let Some(dir) = args.first() else {
        return Err("usage: crow --search-trace DIR QUERY [--target PATH] [--names|--contents] \
                    [--hidden|--no-hidden] [--search-hidden|--no-search-hidden] [--everything]"
            .to_owned());
    };
    let Some(query) = args.get(1) else {
        return Err("crow --search-trace needs a QUERY".to_owned());
    };
    let mut options = TraceOptions {
        dir: PathBuf::from(dir),
        query: query.clone(),
        target: None,
        names: true,
        contents: true,
        show_hidden: false,
        search_includes_hidden: true,
        everything: false,
    };
    let mut hidden_set = false;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--target" => {
                let Some(path) = args.get(index + 1) else {
                    return Err("--target needs a path".to_owned());
                };
                options.target = Some(PathBuf::from(path));
                index += 2;
            }
            "--names" => {
                options.names = true;
                options.contents = false;
                index += 1;
            }
            "--contents" => {
                options.names = false;
                options.contents = true;
                index += 1;
            }
            "--hidden" => {
                options.show_hidden = true;
                hidden_set = true;
                index += 1;
            }
            "--no-hidden" => {
                options.show_hidden = false;
                hidden_set = true;
                index += 1;
            }
            "--search-hidden" => {
                options.search_includes_hidden = true;
                index += 1;
            }
            "--no-search-hidden" => {
                options.search_includes_hidden = false;
                index += 1;
            }
            "--everything" => {
                options.everything = true;
                index += 1;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if !hidden_set {
        // io-ok: CLI diagnostic, before any window opens
        match crow_core::load_settings(&crow_core::config_dir()) {
            Ok(settings) => options.show_hidden = settings.show_hidden_files,
            Err(error) => eprintln!("{error}; using the hidden default"),
        }
    }
    Ok(options)
}

/// Run the real engine and print what it did: each batch, the totals, every
/// skip, and why `--target` was not a hit. No second walk implementation.
fn search_trace(args: &[String]) -> Result<(), String> {
    use std::sync::Arc;

    let options = parse_search_trace(args)?;
    // io-ok: CLI diagnostic, before any window opens
    let root = options.dir.canonicalize().map_err(|e| format!("{}: {e}", options.dir.display()))?;
    let home = crow_core::home();
    // io-ok: CLI diagnostic, before any window opens
    let exclude = match crow_core::load_exclude(&crow_core::config_dir()) {
        Ok(exclude) => exclude,
        Err(error) => {
            eprintln!("{error}; using the default exclude sets");
            crow_core::Exclude::default()
        }
    };
    let rules = crow_search::RuleSet::from_config(&exclude, &home)?;
    let query = crow_search::Query {
        names: options.names.then(|| options.query.clone()),
        contents: options.contents.then(|| options.query.clone()),
        enter_hidden_folders: options.show_hidden,
        include_hidden: options.search_includes_hidden,
        rules: Arc::new(rules),
        everything: options.everything,
    };
    let target = options.target.as_ref().map(|path| {
        let path =
            if path.starts_with("~") { crow_core::expand_home(&path.to_string_lossy(), &home) } else { path.clone() };
        // io-ok: CLI diagnostic, before any window opens
        path.canonicalize().unwrap_or(path)
    });

    let control = crow_search::Control::new();
    let started = std::time::Instant::now();
    let mut batches = 0usize;
    let mut hits = 0usize;
    let mut first_hit = None;
    // The first matching hit: batch number, elapsed time, and the content line.
    let mut verdict: Option<(usize, std::time::Duration, Option<u64>)> = None;
    let summary = crow_search::search(
        &root,
        &query,
        |batch| {
            batches += 1;
            let elapsed = started.elapsed();
            first_hit.get_or_insert(elapsed);
            println!("+{}ms batch {batches}: {} hits", elapsed.as_millis(), batch.len());
            if verdict.is_none()
                && let Some(target) = target.as_ref()
                && let Some(hit) = batch.iter().find(|hit| &hit.path == target)
            {
                let line = match &hit.kind {
                    crow_search::HitKind::Name { .. } => None,
                    crow_search::HitKind::Content { line, .. } => Some(*line),
                };
                verdict = Some((batches, elapsed, line));
            }
            hits += batch.len();
        },
        &control,
    );
    println!(
        "first_hit={}ms total={}ms hits={hits} scanned={} skipped={}",
        first_hit.unwrap_or_default().as_millis(),
        started.elapsed().as_millis(),
        summary.scanned,
        summary.skipped.len()
    );
    for skip in &summary.skipped {
        println!("skip {} ({})", skip.path.display(), skip.reason);
    }
    let Some(target) = target else { return Ok(()) };
    match verdict {
        Some((batch, elapsed, None)) => println!("target: name hit at {}ms (batch {batch})", elapsed.as_millis()),
        Some((_, elapsed, Some(line))) => println!("target: content hit line {line} at {}ms", elapsed.as_millis()),
        None => {
            if !target.starts_with(&root) {
                println!("target: not found; outside {}", options.dir.display());
                return Ok(());
            }
            if let Some(skip) = summary.skipped.iter().find(|skip| target.starts_with(&skip.path)) {
                println!("target: not found; under skipped {} ({})", skip.path.display(), skip.reason);
                return Ok(());
            }
            if options.names {
                let name = target.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
                let matches = crow_search::FuzzyMatcher::new(&options.query).indices(&name).is_some();
                if matches {
                    println!("target: not found; no skipped ancestor, and the name matches (not visited?)");
                } else {
                    println!("target: not found; no skipped ancestor, and the name does not match");
                }
            } else {
                println!("target: not found; no skipped ancestor, and names were not searched");
            }
        }
    }
    Ok(())
}

/// The path in a `file://` URL from LaunchServices, or `None` for other schemes.
fn path_from_open_url(url: &str) -> Option<PathBuf> {
    let url = url::Url::parse(url).ok()?;
    if url.scheme() != "file" {
        return None;
    }
    url.to_file_path().ok()
}

/// Open paths delivered by macOS (`open -a Crow …`, `crow …` through the shim).
/// A folder opens in a new window; a file opens its folder with the file selected.
fn open_paths(urls: Vec<String>, cx: &mut App) {
    let paths: Vec<PathBuf> = urls.iter().filter_map(|url| path_from_open_url(url)).collect();
    if paths.is_empty() {
        return;
    }
    // The main thread never stats: classify in the background, then open on the main thread.
    let task = cx.background_spawn(async move {
        paths
            .into_iter()
            .filter_map(|path| {
                let is_dir = std::fs::metadata(&path).ok()?.is_dir(); // io-ok: inside background_spawn
                Some((path, is_dir))
            })
            .collect::<Vec<_>>()
    });
    cx.spawn(async move |cx| {
        let found = task.await;
        cx.update(|cx| {
            for (path, is_dir) in found {
                tracing::info!(path = %path.display(), "opening from the command line");
                window::open_path(path, is_dir, cx);
            }
        });
    })
    .detach();
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    #[cfg(feature = "fuzz")]
    if args.first().map(String::as_str) == Some("--fuzz") {
        return fuzz_live::run(&args[1..]);
    }
    #[cfg(not(feature = "fuzz"))]
    if args.first().map(String::as_str) == Some("--fuzz") {
        eprintln!("crow --fuzz needs a build with --features fuzz");
        return ExitCode::FAILURE;
    }

    let _log_guard = logging::init();

    if args.first().map(String::as_str) == Some("--make-fixture") {
        let Some(dir) = args.get(1).map(PathBuf::from) else {
            eprintln!("usage: crow --make-fixture DIR [N]");
            return ExitCode::FAILURE;
        };
        let count = args.get(2).and_then(|n| n.parse().ok()).unwrap_or(100_000);
        return match crow_fs::testing::make_tree(&dir, &crow_fs::testing::EdgeTree::wide(count), 0) {
            Ok(_) => {
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

    if args.first().map(String::as_str) == Some("--make-search-corpus") {
        let Some(dir) = args.get(1).map(PathBuf::from) else {
            eprintln!("usage: crow --make-search-corpus DIR");
            return ExitCode::FAILURE;
        };
        return match crow_search::corpus::build(&dir) {
            Ok(()) => {
                println!("created the search corpus in {}", dir.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{}: {e}", dir.display());
                ExitCode::FAILURE
            }
        };
    }

    if args.first().map(String::as_str) == Some("--bench-search") {
        return match bench_search(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }

    if args.first().map(String::as_str) == Some("--search-trace") {
        return match search_trace(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }

    if args.first().map(String::as_str) == Some("--motion-lab") {
        gpui_platform::application().with_assets(icons::Assets).run(|cx: &mut App| {
            effects::install(cx, Arc::new(effects::SystemEffects));
            ui::init(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            motion_lab::open(cx);
            cx.activate(true);
        });
        return ExitCode::SUCCESS;
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
    // io-ok: startup: resolve the folder to open before the window exists
    let start = match start.canonicalize() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("{}: {e}", start.display());
            return ExitCode::FAILURE;
        }
    };

    let app = gpui_platform::application().with_assets(icons::Assets);
    // Paths from `open -a Crow …` arrive as URLs. At launch they can arrive before
    // `run`; the queue is drained there, and the poll below handles a running copy.
    let pending_urls = Arc::new(Mutex::new(Vec::<String>::new()));
    if !bench {
        let pending = pending_urls.clone();
        app.on_open_urls(move |urls| {
            pending.lock().unwrap().extend(urls);
        });
    }
    // Crow stays resident with no windows, like Finder: the Dock icon reopens it.
    app.on_reopen(move |cx| {
        if cx.windows().is_empty() {
            window::open(window::launch_folder(cx), false, None, cx);
        } else {
            let front =
                cx.window_stack().and_then(|stack| stack.first().copied()).or_else(|| cx.windows().first().copied());
            if let Some(handle) = front {
                handle.update(cx, |_, window, _| window.activate_window()).ok();
            }
        }
    });
    app.run(move |cx: &mut App| {
        effects::install(cx, Arc::new(effects::SystemEffects));
        ui::init(cx);
        cx.set_global(window::LaunchFolder(start.clone()));
        extensions::init(cx);
        operations::init(cx, std::sync::Arc::new(crow_macos::SystemTrash));
        cx.bind_keys(actions::finder_keymap());
        cx.bind_keys(actions::vim_keymap());
        cx.bind_keys(text_input::keymap());
        cx.bind_keys(settings_window::keymap());
        menus::init(cx);
        let initial = {
            let mut pending = pending_urls.lock().unwrap();
            std::mem::take(&mut *pending)
        };
        if initial.is_empty() {
            window::open(start.clone(), bench, bench_filter.clone(), cx);
        } else {
            open_paths(initial, cx);
        }
        if !bench {
            let pending = pending_urls.clone();
            cx.spawn(async move |cx| {
                loop {
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                    let urls = {
                        let mut pending = pending.lock().unwrap();
                        std::mem::take(&mut *pending)
                    };
                    if !urls.is_empty() {
                        cx.update(|cx| open_paths(urls, cx));
                    }
                }
            })
            .detach();
        }
        cx.activate(true);
    });
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::path_from_open_url;
    use std::path::PathBuf;

    #[test]
    fn open_urls_become_paths() {
        assert_eq!(path_from_open_url("file:///Users/me/My%20Folder"), Some(PathBuf::from("/Users/me/My Folder")));
        assert_eq!(path_from_open_url("file://localhost/Users/me"), Some(PathBuf::from("/Users/me")));
        assert_eq!(path_from_open_url("file:///"), Some(PathBuf::from("/")));
        assert_eq!(path_from_open_url("https://example.com/a"), None);
        assert_eq!(path_from_open_url("not a URL"), None);
    }
}
