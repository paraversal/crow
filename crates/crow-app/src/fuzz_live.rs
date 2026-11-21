//! `crow --fuzz`: H1's generator in the real app, with a frame-time budget.
//!
//! The test platform has no real frames or disk timing, so `fuzz_smoke` cannot
//! see a freeze. This runner opens a real window on a temp edge tree (or a
//! scratch folder), dispatches generated keys and typing, records a 16 ms
//! main-thread heartbeat (its lateness is the frame budget), and prints the
//! spans that overlapped a slow one. It never opens an app, plays a sound, or
//! touches the user's Trash or pasteboard: effects and Trash are recorded or
//! redirected to temp folders.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui::{App, AppContext, AsyncApp, Keystroke, WindowHandle, px, size};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;

use crate::effects::{self, RecordingEffects};
use crate::fuzz::{Step, StepGen, Violation};
use crate::operations::{self, FolderTrash, Operations};
use crate::window::CrowWindow;
use crate::{actions, extensions, icons, logging, menus, settings_window, text_input, ui, window};
use crow_fs::testing::{Churn, ChurnHandle, EdgeTree, make_tree, remove_tree};

/// A frame over this is a slow step.
const SLOW_MS: f64 = 50.0;
/// A step that acts on more than this many paths is a bulk step: select-all,
/// then copy/cut/paste/trash/undo on a 100k folder. Its budget is `BULK_MS`.
const BULK_PATHS: usize = 10_000;
/// The budget for a bulk step; the 50 ms one is for scroll, navigation, typing
/// and single-item operations.
const BULK_MS: f64 = 100.0;
/// A frame over this fails the run.
const FREEZE_MS: f64 = 250.0;
/// A stall this long is treated as a hang; a watchdog thread quits so a stuck
/// step cannot swallow the rest of the run. A 100k fixture resize can take 5-6 s
/// and still finish, so this is well above the freeze threshold.
const HANG_MS: f64 = 30_000.0;
/// The ordinary fuzz tree's size, matching H1's `fuzz_long`.
const FILES: usize = 200;

#[derive(Clone, Debug)]
struct Options {
    dir: Option<PathBuf>,
    seed: u64,
    minutes: f64,
    until: Option<usize>,
    /// With `--until`: pause this long for a profiler, then keep going.
    pause_ms: u64,
    /// Replay this saved script instead of generating steps.
    script: Option<PathBuf>,
    tree: EdgeTree,
    churn: bool,
}

impl Options {
    fn parse(args: &[String]) -> Result<Options, String> {
        let mut dir = None;
        let mut dir_is_scratch = false;
        let mut seed = default_seed();
        let mut minutes: f64 = 10.0;
        let mut until = None;
        let mut pause_ms = 0;
        let mut script = None;
        let mut tree = EdgeTree::small();
        let mut tree_given = false;
        let mut churn = false;
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--seed" => {
                    let value = args.next().ok_or("--seed needs a number")?;
                    seed = value.parse().map_err(|_| format!("bad --seed {value:?}"))?;
                }
                "--minutes" => {
                    let value = args.next().ok_or("--minutes needs a number")?;
                    minutes = value.parse().map_err(|_| format!("bad --minutes {value:?}"))?;
                    if !minutes.is_finite() || minutes <= 0.0 {
                        return Err("--minutes must be positive".into());
                    }
                }
                "--until" => {
                    let value = args.next().ok_or("--until needs a step index")?;
                    until = Some(value.parse().map_err(|_| format!("bad --until {value:?}"))?);
                }
                "--pause-ms" => {
                    let value = args.next().ok_or("--pause-ms needs milliseconds")?;
                    pause_ms = value.parse().map_err(|_| format!("bad --pause-ms {value:?}"))?;
                }
                "--script" => {
                    let value = args.next().ok_or("--script needs a file")?;
                    script = Some(PathBuf::from(value));
                }
                "--tree" => {
                    let value = args.next().ok_or("--tree needs a kind list")?;
                    tree = EdgeTree::parse(value)?;
                    tree_given = true;
                }
                "--churn" => churn = true,
                "--dir-is-scratch" => dir_is_scratch = true,
                other if other.starts_with("--") => return Err(format!("unknown option {other:?}")),
                path => {
                    if dir.is_some() {
                        return Err(format!("more than one folder: {path:?}"));
                    }
                    dir = Some(PathBuf::from(path));
                }
            }
        }
        if dir.is_some() && !dir_is_scratch {
            return Err("refusing to fuzz a user folder: pass --dir-is-scratch to confirm it may be changed".into());
        }
        if dir.is_some() && tree_given {
            return Err("--tree applies to the temp tree; the given folder is used as-is".into());
        }
        Ok(Options { dir, seed, minutes, until, pause_ms, script, tree, churn })
    }
}

fn default_seed() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
}

/// Options that rebuild the same tree and replay the saved step file. The
/// collector replaces the absolute script path with the report's artifact.
fn build_live_replay(options: &Options, script_path: &Path) -> crate::fuzz_events::Replay {
    let mut command: Vec<String> =
        ["cargo", "run", "--release", "-p", "crow-app", "--features", "fuzz", "--", "--fuzz"]
            .into_iter()
            .map(str::to_owned)
            .collect();
    command.push("--seed".into());
    command.push(options.seed.to_string());
    if let Some(dir) = &options.dir {
        command.push(dir.display().to_string());
        command.push("--dir-is-scratch".into());
    } else {
        command.push("--tree".into());
        command.push(options.tree.spec());
    }
    if options.churn {
        command.push("--churn".into());
    }
    command.push("--script".into());
    command.push(script_path.display().to_string());
    crate::fuzz_events::Replay {
        command,
        env: BTreeMap::new(),
        script: None,
        script_format: crate::fuzz_events::LIVE_FORMAT,
    }
}

fn usage() -> &'static str {
    "crow --fuzz [DIR --dir-is-scratch] [--seed N] [--minutes M] [--churn] [--script FILE] [--until INDEX] [--tree KINDS]

  --fuzz              open a real window and drive it with H1's generator
  DIR                 a scratch folder to fuzz; needs --dir-is-scratch
  --seed N            the generator seed (default: from the clock; printed)
  --minutes M         stop after M minutes (default 10; ignored with --until or --script)
  --script FILE       replay the steps a run saved (its script.txt) instead of generating
  --until INDEX       dispatch steps 0..INDEX-1, then pause for a profiler
  --pause-ms MS       with --until: wait MS for the profiler, then keep going
  --churn             change the tree while the fuzzer acts
  --tree KINDS        temp tree: small (default), all, none, or comma-separated
                      kinds (long-name,emoji,deep-20,wide-100000,huge-file, ...)

A slow step is a 16 ms heartbeat over 50 ms; over 250 ms fails the run.
Every run saves its steps; replay a slow one with the same tree options and
--seed N --script FILE --until INDEX (the report prints the command)."
}

pub fn run(args: &[String]) -> ExitCode {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{}", usage());
        return ExitCode::SUCCESS;
    }
    let options = match Options::parse(args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("crow --fuzz: {error}\n\n{}", usage());
            return ExitCode::FAILURE;
        }
    };
    // Read the replay before building a tree, so a missing script leaves nothing behind.
    let replay = match &options.script {
        Some(path) => match read_script(path) {
            Ok(steps) => Some(Arc::new(steps)),
            Err(error) => {
                eprintln!("crow --fuzz: {error}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };
    let setup = match prepare(&options) {
        Ok(setup) => setup,
        Err(error) => {
            eprintln!("crow --fuzz: {error}");
            return ExitCode::FAILURE;
        }
    };
    // A replay keeps its file; a generated run saves the steps it sends.
    let script_path = options.script.clone().unwrap_or_else(|| setup.support.join("script.txt"));
    let script_log = match &replay {
        Some(_) => None,
        None => match ScriptLog::create(&script_path) {
            Ok(log) => Some(log),
            Err(error) => {
                eprintln!("crow --fuzz: {error}");
                setup.discard();
                return ExitCode::FAILURE;
            }
        },
    };
    let replay_args = format!("--seed {} --script {}", options.seed, script_path.display());
    let seed_replay = build_live_replay(&options, &script_path);
    let trace = Trace::new();
    let _log_guard = logging::init_fuzz(trace.layer(), &setup.log_dir);
    println!(
        "FUZZ seed={} tree={} churn={} minutes={} dir={} config={} log={} script={}",
        options.seed,
        setup.tree_label,
        options.churn,
        options.minutes,
        setup.root.display(),
        setup.config_dir.display(),
        setup.log_dir.join("crow.log").display(),
        script_path.display()
    );
    crate::fuzz_events::seed_start("fuzz-live", options.seed, &setup.tree_label, options.churn, None);
    if let Some(until) = options.until
        && options.pause_ms == 0
    {
        println!("PAUSE before step {until}; the window stays open for profiling");
    }

    let outcome = Arc::new(Mutex::new(None::<Outcome>));
    let app = gpui_platform::application().with_assets(icons::Assets);
    let app_options = options.clone();
    let app_setup = setup.clone();
    let app_trace = trace.clone();
    let app_outcome = outcome.clone();
    let app_replay = seed_replay.clone();
    app.run(move |cx: &mut App| {
        effects::install(cx, Arc::new(RecordingEffects::without_file_urls()));
        ui::init_at(
            cx,
            app_setup.config_dir.clone(),
            app_setup.state_dir.clone(),
            app_setup.root.clone(),
            app_setup.state_dir.join("cache"),
        );
        extensions::init(cx);
        operations::init(cx, Arc::new(FolderTrash(app_setup.trash_dir.clone())));
        crate::plugins::init(cx);
        crate::icon_packs::reload(cx);
        cx.global_mut::<Operations>().jail = Some(app_setup.root.clone());
        cx.bind_keys(actions::finder_keymap());
        cx.bind_keys(actions::vim_keymap());
        cx.bind_keys(text_input::keymap());
        cx.bind_keys(settings_window::keymap());
        menus::init(cx);
        cx.set_global(window::LaunchFolder(app_setup.root.clone()));
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let Some(handle) = window::open(app_setup.root.clone(), false, None, cx) else {
            eprintln!("crow --fuzz: could not open the window");
            crate::fuzz_events::seed_failure_without_counts(
                "fuzz-live",
                app_options.seed,
                0,
                "window",
                "could not open the window",
                &app_replay,
            );
            set_outcome(&app_outcome, Outcome { failed: true });
            app_setup.discard();
            exit_failure();
        };
        if let Err(error) = handle.entity(cx) {
            eprintln!("crow --fuzz: {error}");
            crate::fuzz_events::seed_failure_without_counts(
                "fuzz-live",
                app_options.seed,
                0,
                "window",
                &error.to_string(),
                &app_replay,
            );
            set_outcome(&app_outcome, Outcome { failed: true });
            app_setup.discard();
            exit_failure();
        }
        let frames = Arc::new(Mutex::new(FrameLog::default()));
        install_heartbeat(
            frames.clone(),
            app_trace.clone(),
            app_outcome.clone(),
            app_options.seed,
            app_replay.clone(),
            app_options.script.is_none().then(|| script_path.clone()),
            cx,
        );
        let options = app_options.clone();
        let setup = app_setup.clone();
        let trace = app_trace.clone();
        let source = StepSource { replay: replay.clone(), log: script_log };
        let replay_args = replay_args.clone();
        cx.spawn(async move |cx| {
            let churn = options.churn.then(|| Churn::start(&setup.root, options.seed));
            let result =
                run_steps(handle, &setup.root, &options, source, &frames, &trace, churn.as_ref(), cx).await;
            frames.lock().expect("frames").active = false;
            if let Some(churn) = &churn {
                churn.pause();
            }
            let steps = frames.lock().expect("frames").steps;
            let elapsed = frames.lock().expect("frames").elapsed();
            let slow = !frames.lock().expect("frames").slow.is_empty();
            let (report, stats) =
                frames.lock().expect("frames").report(&options, &setup.tree_label, &replay_args, steps, elapsed);
            print!("{report}");
            crate::fuzz_events::frame_summary(
                options.seed,
                crate::fuzz_events::Frames {
                    heartbeats: stats.heartbeats,
                    threshold_ms: SLOW_MS,
                    over_budget: stats.over_limit,
                    worst_frame_ms: stats.worst_ms,
                    freeze_threshold_ms: FREEZE_MS,
                    freeze: stats.freeze,
                    slow_steps: stats.slow,
                    steps,
                    executed_steps: steps,
                    duration: elapsed,
                },
            );
            match result {
                RunResult::Paused { index } => {
                    println!(
                        "PAUSED before step {index}. The window stays open; attach with:\n  xcrun xctrace record --template 'Time Profiler' --attach crow\nTree: {}\nQuit Crow (⌘Q) when the profile is done.",
                        setup.root.display()
                    );
                    crate::fuzz_events::seed_end("fuzz-live", options.seed, steps, elapsed, "paused");
                    set_outcome(&app_outcome, Outcome { failed: false });
                }
                RunResult::Finished { violations, script } => {
                    let failed = stats.freeze || !violations.is_empty();
                    let failure = violations.first().map(|(index, violation)| {
                        (*index, violation.check.to_owned(), violation.detail.clone())
                    });
                    let failure = failure.or_else(|| {
                        stats.freeze.then(|| {
                            (
                                stats.worst_index.unwrap_or(steps),
                                "freeze".to_owned(),
                                format!("frame over {FREEZE_MS:.0} ms: {:.1} ms", stats.worst_ms),
                            )
                        })
                    });
                    if let Some((index, invariant, detail)) = failure {
                        let mut replay = app_replay.clone();
                        replay.command.push("--until".to_owned());
                        replay.command.push(index.to_string());
                        replay.script = Some(script.iter().map(encode_step).collect::<Vec<_>>().join("\n"));
                        crate::fuzz_events::seed_failure(
                            "fuzz-live",
                            options.seed,
                            index,
                            steps,
                            &invariant,
                            &detail,
                            elapsed,
                            &replay,
                        );
                    }
                    crate::fuzz_events::seed_end(
                        "fuzz-live",
                        options.seed,
                        steps,
                        elapsed,
                        if failed { "failed" } else { "passed" },
                    );
                    if !violations.is_empty() {
                        println!("{} invariant violation(s) (first at step {})", violations.len(), violations[0].0);
                    }
                    set_outcome(&app_outcome, Outcome { failed });
                    cleanup(&setup, failed || slow, cx).await;
                    if failed {
                        exit_failure();
                    }
                    cx.update(|cx| cx.quit());
                }
                RunResult::Failed { index, violation, script } => {
                    let mut replay = app_replay.clone();
                    replay.command.push("--until".to_owned());
                    replay.command.push(index.to_string());
                    replay.script = Some(script.iter().map(encode_step).collect::<Vec<_>>().join("\n"));
                    crate::fuzz_events::seed_failure(
                        "fuzz-live",
                        options.seed,
                        index,
                        index,
                        violation.check,
                        &violation.detail,
                        elapsed,
                        &replay,
                    );
                    println!(
                        "FAIL seed={} step={index}: {} — {}\nreplay: {replay_args} --until {index}\nscript:\n{}",
                        options.seed,
                        violation.check,
                        violation.detail,
                        crate::fuzz::Script(script).print()
                    );
                    set_outcome(&app_outcome, Outcome { failed: true });
                    cleanup(&setup, true, cx).await;
                    exit_failure();
                }
            }
        })
        .detach();
        cx.activate(true);
    });
    match outcome.lock().expect("outcome").take() {
        Some(outcome) if outcome.failed => ExitCode::FAILURE,
        Some(_) => ExitCode::SUCCESS,
        None => {
            setup.discard();
            ExitCode::FAILURE
        }
    }
}

#[derive(Clone)]
struct TreeSetup {
    root: PathBuf,
    config_dir: PathBuf,
    state_dir: PathBuf,
    trash_dir: PathBuf,
    log_dir: PathBuf,
    tree_label: String,
    /// Config, state, Trash, log and the saved script.
    support: PathBuf,
    /// The temp tree, removed after a finished run; a paused run leaves it for
    /// profiling. `None` for a given folder.
    tree: Option<PathBuf>,
}

impl TreeSetup {
    /// Remove the tool-created support folder and tree when a run fails before
    /// its normal `cleanup` can take ownership. A supplied `--dir-is-scratch`
    /// folder has no `tree` entry and is never touched.
    fn discard(&self) {
        let _ = remove_tree(&self.support);
        if let Some(tree) = &self.tree {
            let _ = remove_tree(tree);
        }
    }
}

fn prepare(options: &Options) -> Result<TreeSetup, String> {
    let support = std::env::temp_dir().join(format!("crow-fuzz-support-{}-{}", std::process::id(), options.seed));
    // The support folder and a tool-built tree are removed if `prepare` fails
    // halfway; after a successful return `TreeSetup` and `cleanup` own them.
    // A forced kill runs no destructors, so `crow-fuzz-*` can still be left
    // behind; clean those by exact name, never with a prefix sweep.
    let mut scratch = ScratchGuard::default();
    scratch.paths.push(support.clone());
    let config_dir = support.join("config");
    let state_dir = support.join("state");
    let trash_dir = support.join("trash");
    let log_dir = support.join("logs");
    for dir in [&config_dir, &state_dir, &trash_dir, &log_dir] {
        std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    }

    let (root, tree_label, cleanup_tree) = match &options.dir {
        Some(dir) => {
            if !dir.is_dir() {
                return Err(format!("{}: not a folder", dir.display()));
            }
            let root = dir.canonicalize().map_err(|error| format!("{}: {error}", dir.display()))?;
            (root, "external".to_owned(), None)
        }
        None => {
            let root = std::env::temp_dir().join(format!("crow-fuzz-tree-{}-{}", std::process::id(), options.seed));
            scratch.paths.push(root.clone());
            build_tree(&root, &options.tree, options.seed)?;
            let root = root.canonicalize().map_err(|error| format!("{}: {error}", root.display()))?;
            let cleanup = root.clone();
            (root, options.tree.label(), Some(cleanup))
        }
    };
    let sidebar = sidebar_toml(&root);
    std::fs::write(config_dir.join("sidebar.toml"), sidebar)
        .map_err(|error| format!("{}: {error}", config_dir.join("sidebar.toml").display()))?;
    // The live fuzzer's steps assume a usable window; the first-launch sheet is
    // covered by its own test, not by random input.
    std::fs::write(state_dir.join(crow_core::ONBOARDING_FILE), "{\"tips_choice\": true}\n")
        .map_err(|error| format!("{}: {error}", state_dir.join(crow_core::ONBOARDING_FILE).display()))?;

    scratch.paths.clear();
    Ok(TreeSetup { root, config_dir, state_dir, trash_dir, log_dir, tree_label, support, tree: cleanup_tree })
}

/// Removes tool-created scratch paths unless disarmed on a successful return.
#[derive(Default)]
struct ScratchGuard {
    paths: Vec<PathBuf>,
}

impl Drop for ScratchGuard {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = remove_tree(path);
        }
    }
}

/// The H1 tree: alpha/beta/notes, 200 files, 20 folders, then the edge kinds.
fn build_tree(root: &Path, edge: &EdgeTree, seed: u64) -> Result<(), String> {
    remove_tree(root).map_err(|error| format!("{}: {error}", root.display()))?;
    std::fs::create_dir_all(root.join("alpha/inner")).map_err(|error| format!("{}: {error}", root.display()))?;
    std::fs::create_dir_all(root.join("beta")).map_err(|error| format!("{}: {error}", root.display()))?;
    std::fs::write(root.join("notes.txt"), "hello\n").map_err(|error| format!("{}: {error}", root.display()))?;
    for index in 0..FILES {
        let path = root.join(format!("file-{index:03}.txt"));
        std::fs::write(&path, "x").map_err(|error| format!("{}: {error}", path.display()))?;
    }
    for index in 0..20 {
        let path = root.join(format!("dir-{index:02}/inner"));
        std::fs::create_dir_all(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    if !edge.is_empty() {
        make_tree(root, edge, seed).map_err(|error| format!("{}: {error}", root.display()))?;
    }
    Ok(())
}

fn sidebar_toml(root: &Path) -> String {
    let mut favorites = Vec::new();
    for name in ["alpha", "beta"] {
        let path = root.join(name);
        if path.is_dir() {
            favorites.push(format!("  {{ path = {:?} }},", path.to_string_lossy()));
        }
    }
    if favorites.is_empty() {
        favorites.push(format!("  {{ path = {:?} }},", root.to_string_lossy()));
    }
    format!(
        "[[group]]\nname = \"Favorites\"\nitems = [\n{}\n]\n[recent]\nshow = false\n[locations]\nshow = false\n",
        favorites.join("\n")
    )
}

#[derive(Clone, Copy)]
struct Outcome {
    failed: bool,
}

/// A watchdog freeze must win over a runner result that arrives after it; a
/// successful result must not replace an already-failed one.
fn set_outcome(outcome: &Arc<Mutex<Option<Outcome>>>, value: Outcome) {
    let mut slot = outcome.lock().expect("outcome");
    if slot.as_ref().is_none_or(|current| !current.failed) {
        *slot = Some(value);
    }
}

/// GPUI's macOS `quit` terminates through `NSApplication`, which exits 0 and
/// never returns to `main`; a failed fuzz run has to exit here.
fn exit_failure() -> ! {
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    std::process::exit(1);
}

#[derive(Default)]
struct FrameLog {
    active: bool,
    current: Option<(usize, Step, Instant, bool)>,
    recent: VecDeque<(usize, Step)>,
    slow: Vec<SlowFrame>,
    frames: u64,
    steps: usize,
    started: Option<Instant>,
    /// When the heartbeat last ran; the watchdog thread reads it.
    last_beat: Option<Instant>,
    max_ms: f64,
}

#[derive(Clone)]
struct SlowFrame {
    index: usize,
    step: Step,
    ms: f64,
    /// The step acted on more than `BULK_PATHS` paths; its budget is `BULK_MS`.
    bulk: bool,
}

impl FrameLog {
    fn elapsed(&self) -> Duration {
        self.started.map_or(Duration::ZERO, |started| started.elapsed())
    }

    fn report(
        &self,
        options: &Options,
        tree_label: &str,
        replay_args: &str,
        steps: usize,
        elapsed: Duration,
    ) -> (String, FrameStats) {
        let mut groups: BTreeMap<(&'static str, bool), (usize, f64, usize, Step)> = BTreeMap::new();
        let mut over_limit = 0;
        for slow in &self.slow {
            let limit = if slow.bulk { BULK_MS } else { SLOW_MS };
            if slow.ms > limit {
                over_limit += 1;
            }
            let key = (slow.step.kind(), slow.bulk);
            let entry = groups.entry(key).or_insert((0, 0.0, slow.index, slow.step.clone()));
            entry.0 += 1;
            if slow.ms > entry.1 {
                entry.1 = slow.ms;
                entry.2 = slow.index;
                entry.3 = slow.step.clone();
            }
        }
        let mut rows: Vec<_> = groups.into_iter().collect();
        rows.sort_by(|a, b| b.1.1.total_cmp(&a.1.1));
        let freeze = self.max_ms > FREEZE_MS;
        let mut out = String::new();
        out.push_str(&format!(
            "\n==== crow --fuzz ====\nseed {} · tree {} · churn {} · {} steps in {:.1} s\nheartbeats {} · over {SLOW_MS:.0} ms {} · over the limit {over_limit} · worst {:.1} ms{}\n",
            options.seed,
            tree_label,
            options.churn,
            steps,
            elapsed.as_secs_f64(),
            self.frames,
            self.slow.len(),
            self.max_ms,
            if freeze { " · FREEZE" } else { "" }
        ));
        if rows.is_empty() {
            out.push_str("no step over the budget\n");
        } else {
            out.push_str(&format!("{:<20} {:>5} {:>10} {:>7}  replay\n", "step", "over", "worst", "limit"));
            for ((kind, bulk), (count, worst, index, step)) in rows {
                let kind = if bulk { format!("{kind} (bulk)") } else { kind.to_owned() };
                let limit = if bulk { BULK_MS } else { SLOW_MS };
                out.push_str(&format!(
                    "{kind:<20} {count:>5} {worst:>8.1} ms {limit:>6.0} ms  {replay_args} --until {index}  {step:?}\n"
                ));
            }
        }
        (
            out,
            FrameStats {
                freeze,
                over_limit,
                heartbeats: self.frames,
                slow: self.slow.len(),
                worst_ms: self.max_ms,
                worst_index: self.slow.iter().max_by(|a, b| a.ms.total_cmp(&b.ms)).map(|slow| slow.index),
            },
        )
    }
}

/// Totals the machine event needs, computed while building the human report.
#[derive(Clone, Copy, Debug, Default)]
struct FrameStats {
    freeze: bool,
    over_limit: usize,
    heartbeats: u64,
    slow: usize,
    worst_ms: f64,
    worst_index: Option<usize>,
}

/// Whether `step` acts on more than `BULK_PATHS` paths, so its budget is
/// `BULK_MS` rather than `SLOW_MS`. Read before the step runs: select-all and
/// paste grow the selection or the clipboard during the step.
fn is_bulk_step(step: &Step, handle: WindowHandle<CrowWindow>, cx: &mut AsyncApp) -> bool {
    let Ok((selected, rows, clipboard, undo, redo)) = handle.read_with(cx, |view, cx| view.fuzz_path_counts(cx)) else {
        return false;
    };
    match step {
        Step::Keys(keys) if keys == "cmd-a" => rows > BULK_PATHS,
        Step::Keys(keys) if keys == "cmd-v" || keys == "cmd-option-v" || keys == "p" => clipboard > BULK_PATHS,
        Step::Keys(keys) if keys == "cmd-z" => undo > BULK_PATHS,
        Step::Keys(keys) if keys == "cmd-shift-z" => redo > BULK_PATHS,
        // Only the keys that act on the whole selection are bulk. Navigation,
        // typing and clicks keep the 50 ms budget even with a 100k selection.
        Step::Keys(keys)
            if matches!(
                keys.as_str(),
                "cmd-c" | "cmd-x" | "cmd-d" | "cmd-backspace" | "cmd-alt-backspace" | "y y" | "d d"
            ) =>
        {
            selected > BULK_PATHS
        }
        _ => false,
    }
}

/// A 16 ms heartbeat on the main thread. Its lateness is the frame budget: a
/// step that blocks the thread delays the timer that would otherwise wake it.
fn install_heartbeat(
    frames: Arc<Mutex<FrameLog>>,
    trace: Arc<Trace>,
    outcome: Arc<Mutex<Option<Outcome>>>,
    seed: u64,
    replay: crate::fuzz_events::Replay,
    generated_script: Option<PathBuf>,
    cx: &mut App,
) {
    {
        let mut log = frames.lock().expect("frames");
        log.started = Some(Instant::now());
        log.last_beat = Some(Instant::now());
        log.active = true;
    }
    install_watchdog(frames.clone(), trace.clone(), seed, replay, generated_script);
    cx.spawn(async move |cx| {
        let mut last = Instant::now();
        let mut frozen_steps = std::collections::HashSet::new();
        loop {
            cx.background_executor().timer(Duration::from_millis(16)).await;
            let now = Instant::now();
            let ms = (now - last).as_secs_f64() * 1000.0;
            last = now;
            let mut freeze = None;
            let report = {
                let mut log = frames.lock().expect("frames");
                if !log.active {
                    break;
                }
                log.last_beat = Some(now);
                if let Some((index, step, started, bulk)) = log.current.clone() {
                    log.frames += 1;
                    log.max_ms = log.max_ms.max(ms);
                    if ms > SLOW_MS {
                        let first = !log.slow.iter().any(|slow| slow.index == index);
                        if let Some(slow) = log.slow.iter_mut().find(|slow| slow.index == index) {
                            slow.ms = slow.ms.max(ms);
                        } else {
                            log.slow.push(SlowFrame { index, step: step.clone(), ms, bulk });
                        }
                        if ms > FREEZE_MS {
                            freeze = Some((
                                index,
                                step.clone(),
                                started,
                                ms,
                                log.recent.iter().cloned().collect::<Vec<_>>(),
                            ));
                        }
                        first.then(|| (index, step, started, ms, log.recent.iter().cloned().collect::<Vec<_>>()))
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some((index, step, started, ms, recent)) = report {
                println!("SLOW seed-step {index} {ms:.1} ms {}", stall_detail(&trace, &step, started, now, &recent));
            }
            if let Some((index, step, started, ms, recent)) = freeze
                && frozen_steps.insert(index)
            {
                println!(
                    "FREEZE seed={seed} step={index} {ms:.1} ms {}",
                    stall_detail(&trace, &step, started, now, &recent)
                );
                *outcome.lock().expect("outcome") = Some(Outcome { failed: true });
            }
        }
    })
    .detach();
}

/// The heartbeat runs on the main thread, so it cannot see a step that never
/// returns. This thread can: when the heartbeat has not run for `HANG_MS` while
/// a step is current, it prints `HANG` and exits 1.
fn install_watchdog(
    frames: Arc<Mutex<FrameLog>>,
    trace: Arc<Trace>,
    seed: u64,
    replay: crate::fuzz_events::Replay,
    generated_script: Option<PathBuf>,
) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let (index, step, started, last_beat, recent) = {
                let log = frames.lock().expect("frames");
                if !log.active {
                    return;
                }
                let (Some((index, step, started, _)), Some(last_beat)) = (log.current.clone(), log.last_beat) else {
                    continue;
                };
                (index, step, started, last_beat, log.recent.iter().cloned().collect::<Vec<_>>())
            };
            let now = Instant::now();
            let ms = (now - last_beat).as_secs_f64() * 1000.0;
            if ms > HANG_MS {
                println!(
                    "HANG seed={seed} step={index} {ms:.1} ms and counting {}",
                    stall_detail(&trace, &step, started, now, &recent)
                );
                let mut replay = replay.clone();
                replay.command.push("--until".to_owned());
                replay.command.push(index.to_string());
                // A generated run's tool-owned script is kept for replay; a
                // user-supplied --script is not copied into the report.
                if let Some(path) = &generated_script
                    && let Ok(text) = std::fs::read_to_string(path)
                {
                    replay.script = Some(text);
                }
                crate::fuzz_events::seed_failure_without_counts(
                    "fuzz-live",
                    seed,
                    index,
                    "hang",
                    &format!("no heartbeat for {ms:.1} ms"),
                    &replay,
                );
                exit_failure();
            }
        }
    });
}

/// The step, the five before it and the spans that overlapped the stall.
fn stall_detail(trace: &Trace, step: &Step, started: Instant, now: Instant, recent: &[(usize, Step)]) -> String {
    let spans = trace.span_summary(started, now);
    let recent: Vec<String> = recent.iter().map(|(index, step)| format!("{index}:{step:?}")).collect();
    format!(
        "{} {:?}\n  last: {}\n  spans: {}",
        step.kind(),
        step,
        if recent.is_empty() { "none".to_owned() } else { recent.join(", ") },
        if spans.is_empty() { "none (RUST_LOG may hide debug spans)".to_owned() } else { spans.join(", ") }
    )
}

enum RunResult {
    Finished { violations: Vec<(usize, Violation)>, script: Vec<Step> },
    Paused { index: usize },
    Failed { index: usize, violation: Violation, script: Vec<Step> },
}

#[allow(clippy::too_many_arguments)] // the runner's whole context; a struct would only rename it
async fn run_steps(
    handle: WindowHandle<CrowWindow>,
    tree: &Path,
    options: &Options,
    source: StepSource,
    frames: &Arc<Mutex<FrameLog>>,
    trace: &Arc<Trace>,
    churn: Option<&ChurnHandle>,
    cx: &mut AsyncApp,
) -> RunResult {
    let started = Instant::now();
    let deadline = Duration::from_secs_f64(options.minutes * 60.0);
    let mut generator = StepGen::new(options.seed);
    let mut script = Vec::new();
    let mut violations = Vec::new();
    let mut seen = HashSet::new();
    let mut executed = 0usize;
    let mut paused = false;
    // The opening listing and first paint are startup, not step 0; H1's runner
    // settles the window before its first step too.
    if let Some(churn) = churn {
        churn.pause();
    }
    wait_quiet(handle, cx).await;
    clear_recorded_effects(cx);
    if let Some(churn) = churn {
        churn.resume();
    }
    for index in 0.. {
        if !paused && options.until.is_some_and(|until| index >= until) {
            if let Some(churn) = churn {
                churn.pause();
            }
            wait_quiet(handle, cx).await;
            if options.pause_ms == 0 {
                return RunResult::Paused { index };
            }
            // A sampler attaches now and keeps sampling while the steps run. The
            // pause itself is not a step: clear the current frame so the
            // heartbeat does not count it as a slow one.
            println!("PAUSE before step {index} for {} ms; attach the profiler now", options.pause_ms);
            frames.lock().expect("frames").current = None;
            cx.background_executor().timer(Duration::from_millis(options.pause_ms)).await;
            paused = true;
            if let Some(churn) = churn {
                churn.resume();
            }
        }
        // A plain run stops at its deadline; one that resumed after a profiling
        // pause does too, since `--until` no longer ends it.
        if source.replay.is_none() && started.elapsed() >= deadline && (options.until.is_none() || paused) {
            break;
        }
        let step = match &source.replay {
            // A replay sends the saved steps as they are: the generator reads the
            // live keymap and focus, which depend on timing, so a seed alone can
            // drift from the run it came from.
            Some(steps) => match steps.get(index) {
                Some(step) => step.clone(),
                None => break,
            },
            None => match next_step(&mut generator, handle, cx) {
                Some(step) => step,
                None => {
                    return RunResult::Failed {
                        index,
                        violation: Violation::new("window", "the fuzz window closed"),
                        script,
                    };
                }
            },
        };
        if let Some(log) = &source.log {
            log.push(&step);
        }
        script.push(step.clone());
        let bulk = is_bulk_step(&step, handle, cx);
        {
            let mut log = frames.lock().expect("frames");
            log.current = Some((index, step.clone(), Instant::now(), bulk));
            log.recent.push_back((index, step.clone()));
            if log.recent.len() > 5 {
                log.recent.pop_front();
            }
        }
        trace.reset_errors();
        if let Err(violation) = apply_step(handle, tree, churn, frames, &step, cx).await {
            if matches!(violation.check, "panic" | "window" | "dispatch" | "jail") {
                return RunResult::Failed { index, violation, script };
            }
            note_violation(options.seed, index, violation, &mut seen, &mut violations);
        }
        // The recorder is a test observation buffer; a live run never reads it,
        // and a select-all copy records 100k paths per call (findings #14).
        clear_recorded_effects(cx);
        // One step per frame at a realistic pace. Let deferred focus and render
        // work run before checking, as H1's run_until_parked does.
        cx.background_executor().timer(Duration::from_millis(16)).await;
        executed = index + 1;
        frames.lock().expect("frames").steps = executed;
        // H1 checks after every step; H3 checks every 25th, because the checks
        // themselves are main-thread work that would show up as a slow step.
        if index % 25 == 0 {
            frames.lock().expect("frames").current = None;
            if let Some(violation) = check_windows(tree, cx) {
                if violation.check == "jail" {
                    return RunResult::Failed { index, violation, script };
                }
                note_violation(options.seed, index, violation, &mut seen, &mut violations);
            }
        }
        if trace.errors() > 0 {
            let violation = Violation::new("log", format!("{} error-level log line(s)", trace.errors()));
            note_violation(options.seed, index, violation, &mut seen, &mut violations);
        }
    }

    if let Some(churn) = churn {
        churn.pause();
    }
    wait_quiet(handle, cx).await;
    frames.lock().expect("frames").current = None;
    if let Some(violation) = check_windows(tree, cx) {
        note_violation(options.seed, executed, violation, &mut seen, &mut violations);
    }
    if let Some(violation) = deep_violation(cx, tree).await {
        note_violation(options.seed, executed, violation, &mut seen, &mut violations);
    }
    RunResult::Finished { violations, script }
}

fn note_violation(
    seed: u64,
    index: usize,
    violation: Violation,
    seen: &mut HashSet<(&'static str, String)>,
    violations: &mut Vec<(usize, Violation)>,
) {
    let key = (violation.check, violation.detail.clone());
    if seen.insert(key) {
        println!("VIOLATION seed={seed} step={index}: {} — {}", violation.check, violation.detail);
        violations.push((index, violation));
    }
}

async fn apply_step(
    handle: WindowHandle<CrowWindow>,
    tree: &Path,
    churn: Option<&ChurnHandle>,
    frames: &Arc<Mutex<FrameLog>>,
    step: &Step,
    cx: &mut AsyncApp,
) -> Result<(), Violation> {
    match step {
        Step::Keys(keys) => dispatch_keys(handle, keys, cx)?,
        Step::Type(text) => dispatch_type(handle, text, cx)?,
        Step::Resize(width, height) => {
            let resized = cx.update(|cx| {
                cx.update_window(handle.into(), |_, window, _| window.resize(size(px(*width), px(*height))))
            });
            if resized.is_err() {
                return Err(Violation::new("window", "the fuzz window closed"));
            }
        }
        Step::Settle => {
            if let Some(churn) = churn {
                churn.pause();
            }
            wait_quiet(handle, cx).await;
            // The snapshot/compare is the harness reading a 100k listing; do not
            // report its time as a slow app step.
            frames.lock().expect("frames").current = None;
            let found = deep_violation(cx, tree).await;
            if let Some(churn) = churn {
                churn.resume();
            }
            if let Some(violation) = found {
                return Err(violation);
            }
        }
        Step::DeleteShownFolder => {
            let dir = shown_dir(handle, cx)?;
            // Keep the fuzz target's own root: deleting it would end a 100k run.
            // H1 covers a deleted root in the test platform.
            if dir != tree && dir.starts_with(tree) {
                cx.background_executor()
                    .spawn(async move {
                        let _ = remove_tree(&dir);
                    })
                    .await;
            }
        }
        Step::ReplaceShownFolder => {
            let dir = shown_dir(handle, cx)?;
            if dir != tree && dir.starts_with(tree) {
                cx.background_executor()
                    .spawn(async move {
                        let _ = remove_tree(&dir);
                        let _ = std::fs::create_dir_all(&dir);
                        for index in 0..3 {
                            let _ = std::fs::write(dir.join(format!("replaced-{index}.txt")), "replaced\n");
                        }
                    })
                    .await;
            }
        }
        // AdvanceClock is a test-clock step; real timers run at the 16 ms pace.
        // Clicks are off for H3's first version (Q3).
        Step::AdvanceClock(_) | Step::Click(_) | Step::RightClick(..) | Step::ClickWith(..) | Step::Drag(..) => {}
    }
    bring_back(handle, tree, cx).await?;
    Ok(())
}

fn shown_dir(handle: WindowHandle<CrowWindow>, cx: &mut AsyncApp) -> Result<PathBuf, Violation> {
    handle.update(cx, |view, _, cx| view.current_dir(cx)).map_err(|error| Violation::new("window", error.to_string()))
}

async fn bring_back(handle: WindowHandle<CrowWindow>, tree: &Path, cx: &mut AsyncApp) -> Result<(), Violation> {
    let dir = shown_dir(handle, cx)?;
    if !dir.starts_with(tree) {
        handle
            .update(cx, |view, _, cx| view.navigate_to(tree.to_path_buf(), cx))
            .map_err(|error| Violation::new("window", error.to_string()))?;
        wait_quiet(handle, cx).await;
    }
    Ok(())
}

/// The generator's next step for the window as it is now; `None` if it closed.
fn next_step(generator: &mut StepGen, handle: WindowHandle<CrowWindow>, cx: &mut AsyncApp) -> Option<Step> {
    let mut state = generator_state(handle, cx)?;
    // Q3: the first version drives keys and typing only. An empty selector
    // list makes the generator fall back to keys for its click rolls.
    state.selectors.clear();
    // Hiding the app leaves the window hidden while the run keeps driving
    // it; the 100k fixture then stalls on the next step. H1's test platform
    // has no hide implementation, so this is H3-only.
    state.active_keys.retain(|key| !matches!(key.as_str(), "cmd-h" | "alt-cmd-h" | "cmd-m" | "cmd-q"));
    state.all_keys.retain(|key| !matches!(key.as_str(), "cmd-h" | "alt-cmd-h" | "cmd-m" | "cmd-q"));
    Some(generator.next(&state))
}

fn generator_state(handle: WindowHandle<CrowWindow>, cx: &mut AsyncApp) -> Option<crate::fuzz::GenState> {
    handle.update(cx, |view, window, cx| crate::fuzz::state_for(view, window, cx)).ok()
}

fn dispatch_keys(handle: WindowHandle<CrowWindow>, keys: &str, cx: &mut AsyncApp) -> Result<(), Violation> {
    let os = cx.update(|cx| cx.platform_info().os);
    cx.update(|cx| {
        cx.update_window(handle.into(), |_, window, cx| {
            for part in keys.split(' ') {
                let keystroke = Keystroke::parse_with_os(part, os)
                    .map_err(|error| Violation::new("dispatch", format!("{part:?}: {error}")))?;
                window.dispatch_keystroke(keystroke, cx);
            }
            Ok::<(), Violation>(())
        })
    })
    .map_err(|error| Violation::new("window", error.to_string()))?
}

/// Drop the effects the live run recorded: nothing reads them here.
fn clear_recorded_effects(cx: &mut AsyncApp) {
    cx.update(|cx| {
        if let Some(recorder) = crate::effects::recording(cx) {
            recorder.clear();
        }
    });
}

fn dispatch_type(handle: WindowHandle<CrowWindow>, text: &str, cx: &mut AsyncApp) -> Result<(), Violation> {
    let os = cx.update(|cx| cx.platform_info().os);
    cx.update(|cx| {
        cx.update_window(handle.into(), |_, window, cx| {
            for character in text.chars() {
                let keystroke = Keystroke::parse_with_os(&character.to_string(), os)
                    .map_err(|error| Violation::new("dispatch", format!("{character:?}: {error}")))?;
                window.dispatch_keystroke(keystroke, cx);
            }
            Ok::<(), Violation>(())
        })
    })
    .map_err(|error| Violation::new("window", error.to_string()))?
}

async fn wait_quiet(handle: WindowHandle<CrowWindow>, cx: &mut AsyncApp) {
    let mut quiet = 0;
    for _ in 0..200 {
        cx.background_executor().timer(Duration::from_millis(50)).await;
        let busy = handle.update(cx, |view, _, cx| view.is_busy(cx)).unwrap_or(true);
        if busy {
            quiet = 0;
        } else {
            quiet += 1;
            if quiet >= 2 {
                return;
            }
        }
    }
}

fn check_windows(tree: &Path, cx: &mut AsyncApp) -> Option<Violation> {
    for handle in cx.update(|cx| cx.windows()) {
        let Some(handle) = handle.downcast::<CrowWindow>() else { continue };
        if let Ok(found) = handle.update(cx, |view, window, cx| {
            view.check_invariants_with_focus(window, cx, false, Some(tree), window.is_window_active())
        }) && let Some(violation) = found.into_iter().next()
        {
            return Some(violation);
        }
    }
    None
}

async fn deep_violation(cx: &mut AsyncApp, tree: &Path) -> Option<Violation> {
    let mut last = None;
    for _ in 0..8 {
        let mut snapshots = Vec::new();
        for handle in cx.update(|cx| cx.windows()) {
            let Some(handle) = handle.downcast::<CrowWindow>() else { continue };
            if let Ok(found) = handle.update(cx, |view, _, cx| view.listing_snapshot(cx, Some(tree))) {
                snapshots.extend(found);
            }
        }
        let found = cx.background_executor().spawn(async move { compare_snapshots(snapshots) }).await;
        match found {
            Some(violation) => last = Some(violation),
            None => return None,
        }
        cx.background_executor().timer(Duration::from_millis(40)).await;
    }
    last
}

fn compare_snapshots(snapshots: Vec<(PathBuf, Vec<String>, bool)>) -> Option<Violation> {
    for (index, (dir, mut listed, hidden)) in snapshots.into_iter().enumerate() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        let mut disk: Vec<String> = read
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| hidden || !name.starts_with('.'))
            .collect();
        disk.sort();
        listed.sort();
        if disk != listed {
            return Some(Violation::new(
                "listing",
                format!("pane {index} {}: disk {disk:?}, listing {listed:?}", dir.display()),
            ));
        }
    }
    None
}

/// Remove the temp tree. The support folder (script, log) is removed too unless
/// the run has something to replay.
async fn cleanup(setup: &TreeSetup, keep_support: bool, cx: &mut AsyncApp) {
    let mut paths: Vec<PathBuf> = setup.tree.iter().cloned().collect();
    if keep_support {
        println!("kept for replay: {}", setup.support.display());
    } else {
        paths.push(setup.support.clone());
    }
    cx.background_executor()
        .spawn(async move {
            for path in paths {
                let _ = remove_tree(&path);
            }
        })
        .await;
}

/// Where a run's steps come from and where they are saved.
struct StepSource {
    replay: Option<Arc<Vec<Step>>>,
    log: Option<ScriptLog>,
}

/// Saves each step before it is sent, one line per step, from a writer thread:
/// no disk I/O on the main thread, and a `HANG` or a killed run still has the
/// step that stalled.
struct ScriptLog(std::sync::mpsc::Sender<String>);

impl ScriptLog {
    fn create(path: &Path) -> Result<ScriptLog, String> {
        let file = std::fs::File::create(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let (send, receive) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut file = std::io::LineWriter::new(file);
            for line in receive {
                if writeln!(file, "{line}").is_err() {
                    return;
                }
            }
        });
        Ok(ScriptLog(send))
    }

    fn push(&self, step: &Step) {
        let _ = self.0.send(encode_step(step));
    }
}

/// One step per line: the variant, then its fields. Text is the rest of the
/// line, with `\\`, `\n` and `\r` escaped, so spaces and any Unicode survive.
fn encode_step(step: &Step) -> String {
    match step {
        Step::Keys(keys) => format!("Keys {}", escape(keys)),
        Step::Type(text) => format!("Type {}", escape(text)),
        Step::Click(selector) => format!("Click {}", escape(selector)),
        Step::RightClick(selector, slot) => {
            format!("RightClick {} {}", slot.map_or("-".to_owned(), |slot| slot.to_string()), escape(selector))
        }
        Step::ClickWith(selector, mods) => format!("ClickWith {mods} {}", escape(selector)),
        Step::Drag(from, to, mods) => format!("Drag {mods} {}\u{1f}{}", escape(from), escape(to)),
        Step::Resize(width, height) => format!("Resize {width} {height}"),
        Step::Settle => "Settle".to_owned(),
        Step::AdvanceClock(ms) => format!("AdvanceClock {ms}"),
        Step::DeleteShownFolder => "DeleteShownFolder".to_owned(),
        Step::ReplaceShownFolder => "ReplaceShownFolder".to_owned(),
    }
}

fn decode_step(line: &str) -> Result<Step, String> {
    let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
    let number = |value: &str| value.parse::<f64>().map_err(|_| format!("bad number {value:?} in {line:?}"));
    Ok(match kind {
        "Keys" => Step::Keys(unescape(rest)),
        "Type" => Step::Type(unescape(rest)),
        "Click" => Step::Click(unescape(rest)),
        "RightClick" => {
            let (slot, selector) = rest.split_once(' ').ok_or_else(|| format!("bad step {line:?}"))?;
            let slot = if slot == "-" { None } else { Some(number(slot)? as usize) };
            Step::RightClick(unescape(selector), slot)
        }
        "ClickWith" => {
            let (mods, selector) = rest.split_once(' ').ok_or_else(|| format!("bad step {line:?}"))?;
            Step::ClickWith(unescape(selector), number(mods)? as u8)
        }
        "Drag" => {
            let (mods, paths) = rest.split_once(' ').ok_or_else(|| format!("bad step {line:?}"))?;
            let (from, to) = paths.split_once('\u{1f}').ok_or_else(|| format!("bad step {line:?}"))?;
            Step::Drag(unescape(from), unescape(to), number(mods)? as u8)
        }
        "Resize" => {
            let (width, height) = rest.split_once(' ').ok_or_else(|| format!("bad step {line:?}"))?;
            Step::Resize(number(width)? as f32, number(height)? as f32)
        }
        "Settle" => Step::Settle,
        "AdvanceClock" => Step::AdvanceClock(number(rest)? as u64),
        "DeleteShownFolder" => Step::DeleteShownFolder,
        "ReplaceShownFolder" => Step::ReplaceShownFolder,
        _ => return Err(format!("bad step {line:?}")),
    })
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r")
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// A saved script, read before the window opens.
fn read_script(path: &Path) -> Result<Vec<Step>, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    text.lines()
        .filter(|line| !line.is_empty())
        .map(decode_step)
        .collect::<Result<_, _>>()
        .map_err(|error| format!("{}: {error}", path.display()))
}

/// What the slow-step reporter saw: closed spans in a window, plus error logs.
pub struct Trace {
    spans: Mutex<SpanStore>,
    errors: AtomicUsize,
}

#[derive(Default)]
struct SpanStore {
    open: HashMap<u64, SpanRecord>,
    closed: VecDeque<SpanRecord>,
}

struct SpanRecord {
    name: &'static str,
    start: Instant,
    end: Option<Instant>,
}

const MAX_SPANS: usize = 4096;

impl Trace {
    pub fn new() -> Arc<Self> {
        Arc::new(Trace { spans: Mutex::new(SpanStore::default()), errors: AtomicUsize::new(0) })
    }

    pub fn layer(self: &Arc<Self>) -> TraceLayer {
        TraceLayer { trace: self.clone() }
    }

    fn reset_errors(&self) {
        self.errors.store(0, Ordering::Relaxed);
    }

    fn errors(&self) -> usize {
        self.errors.load(Ordering::Relaxed)
    }

    fn span_summary(&self, start: Instant, end: Instant) -> Vec<String> {
        let store = self.spans.lock().expect("spans");
        let mut spans: Vec<(&'static str, f64, bool)> = store
            .closed
            .iter()
            .filter(|span| span.start <= end && span.end.is_some_and(|closed| closed >= start))
            .map(|span| (span.name, (span.end.expect("closed") - span.start).as_secs_f64() * 1000.0, false))
            .chain(
                store
                    .open
                    .values()
                    .filter(|span| span.start <= end)
                    .map(|span| (span.name, (end - span.start).as_secs_f64() * 1000.0, true)),
            )
            .collect();
        spans.sort_by(|a, b| b.1.total_cmp(&a.1));
        spans.truncate(8);
        spans
            .into_iter()
            .map(|(name, ms, open)| if open { format!("{name} {ms:.1}ms (open)") } else { format!("{name} {ms:.1}ms") })
            .collect()
    }
}

/// The tracing layer that feeds [`Trace`]. Installed by `logging::init_fuzz`.
pub struct TraceLayer {
    trace: Arc<Trace>,
}

impl<S: Subscriber> Layer<S> for TraceLayer {
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let record = SpanRecord { name: attrs.metadata().name(), start: Instant::now(), end: None };
        self.trace.spans.lock().expect("spans").open.insert(id.into_u64(), record);
    }

    fn on_close(&self, id: tracing::span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let mut store = self.trace.spans.lock().expect("spans");
        let Some(mut span) = store.open.remove(&id.into_u64()) else { return };
        span.end = Some(Instant::now());
        store.closed.push_back(span);
        if store.closed.len() > MAX_SPANS {
            store.closed.pop_front();
        }
    }

    fn on_event(&self, event: &Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        if event.metadata().level() == &Level::ERROR {
            self.trace.errors.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_need_the_scratch_confirmation() {
        let error = Options::parse(&["/tmp/fuzz".into()]).expect_err("a plain folder is refused");
        assert!(error.contains("--dir-is-scratch"), "{error}");
    }

    #[test]
    fn options_parse_a_seeded_run() {
        let options =
            Options::parse(&["--seed".into(), "7".into(), "--minutes".into(), "0.5".into(), "--churn".into()])
                .expect("options");
        assert_eq!(options.seed, 7);
        assert_eq!(options.minutes, 0.5);
        assert!(options.churn);
        assert_eq!(options.tree, EdgeTree::small());
    }

    #[test]
    fn options_reject_a_tree_with_a_scratched_folder() {
        let error = Options::parse(&["--dir-is-scratch".into(), "--tree".into(), "none".into(), "/tmp/fuzz".into()])
            .expect_err("the tree option builds the temp tree");
        assert!(error.contains("--tree"), "{error}");
    }

    #[test]
    fn discard_removes_tool_paths_but_not_a_supplied_tree() {
        use std::os::unix::fs::PermissionsExt;

        let parent = tempfile::Builder::new().prefix("crow-fuzz-discard-").tempdir().expect("discard parent");
        let support = parent.path().join("support");
        let tree = parent.path().join("tree");
        let supplied = parent.path().join("supplied");
        for path in [&support, &tree, &supplied] {
            std::fs::create_dir_all(path).expect("create discard fixture");
        }
        let locked = tree.join("locked");
        std::fs::create_dir_all(&locked).expect("create locked folder");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("lock folder");
        let setup = TreeSetup {
            root: supplied.clone(),
            config_dir: support.join("config"),
            state_dir: support.join("state"),
            trash_dir: support.join("trash"),
            log_dir: support.join("logs"),
            tree_label: "discard".into(),
            support: support.clone(),
            tree: Some(tree.clone()),
        };
        setup.discard();
        assert!(!support.exists(), "the tool-created support folder is removed");
        assert!(!tree.exists(), "the mode-000 tree is restored and removed");
        assert!(supplied.exists(), "a supplied scratch folder is never touched");
    }

    #[test]
    fn frame_report_groups_by_step_kind() {
        let log = FrameLog {
            slow: vec![
                SlowFrame { index: 3, step: Step::Keys("down".into()), ms: 61.0, bulk: false },
                SlowFrame { index: 8, step: Step::Keys("up".into()), ms: 93.0, bulk: false },
                SlowFrame { index: 9, step: Step::Settle, ms: 71.0, bulk: true },
            ],
            max_ms: 93.0,
            frames: 12,
            ..Default::default()
        };
        let options = Options {
            dir: None,
            seed: 1,
            minutes: 1.0,
            until: None,
            pause_ms: 0,
            script: None,
            tree: EdgeTree::small(),
            churn: false,
        };
        let (report, stats) = log.report(&options, "small", "--seed 1 --script s.txt", 10, Duration::from_secs(1));
        assert!(!stats.freeze);
        assert!(report.contains("Keys"), "{report}");
        assert!(report.contains("Settle"), "{report}");
        assert!(report.contains("--seed 1 --script s.txt --until 8"), "{report}");
    }

    #[test]
    fn the_report_tags_bulk_steps_and_counts_their_limit() {
        let log = FrameLog {
            slow: vec![
                SlowFrame { index: 3, step: Step::Keys("cmd-a".into()), ms: 61.0, bulk: false },
                SlowFrame { index: 4, step: Step::Settle, ms: 80.0, bulk: true },
            ],
            max_ms: 80.0,
            frames: 2,
            ..Default::default()
        };
        let options = Options {
            dir: None,
            seed: 1,
            minutes: 1.0,
            until: None,
            pause_ms: 0,
            script: None,
            tree: EdgeTree::small(),
            churn: false,
        };
        let (report, stats) = log.report(&options, "small", "--seed 1", 10, Duration::from_secs(1));
        assert!(!stats.freeze);
        // The 61 ms non-bulk frame is over its 50 ms limit; the 80 ms bulk one is
        // under its 100 ms budget.
        assert!(report.contains("over the limit 1"), "{report}");
        assert!(report.contains("Settle (bulk)"), "{report}");
        assert!(report.contains("100 ms"), "{report}");
    }

    #[test]
    fn a_saved_script_reads_back_step_for_step() {
        let steps = vec![
            Step::Keys("cmd-shift-]".into()),
            Step::Keys("space".into()),
            Step::Type("e\u{301}9 \\n \n\r🙂 ש".into()),
            Step::Click("row-a b".into()),
            Step::RightClick("row-x".into(), Some(3)),
            Step::RightClick("row-y".into(), None),
            Step::Resize(1920.0, 1100.5),
            Step::Settle,
            Step::AdvanceClock(1082),
            Step::DeleteShownFolder,
            Step::ReplaceShownFolder,
        ];
        let text: String = steps.iter().map(|step| encode_step(step) + "\n").collect();
        assert_eq!(text.lines().count(), steps.len(), "one line per step:\n{text}");
        let read: Vec<Step> = text.lines().map(|line| decode_step(line).expect("decodes")).collect();
        assert_eq!(read, steps);
    }

    #[test]
    fn options_parse_a_script_replay() {
        let options = Options::parse(&[
            "--seed".into(),
            "7".into(),
            "--script".into(),
            "/tmp/s.txt".into(),
            "--until".into(),
            "40".into(),
        ])
        .expect("options");
        assert_eq!(options.script.as_deref(), Some(Path::new("/tmp/s.txt")));
        assert_eq!(options.until, Some(40));
        assert_eq!(options.pause_ms, 0);
        assert!(decode_step("Bogus 1").is_err());
    }

    #[test]
    fn a_failed_outcome_is_sticky() {
        let outcome = Arc::new(Mutex::new(None));
        set_outcome(&outcome, Outcome { failed: true });
        set_outcome(&outcome, Outcome { failed: false });
        assert!(outcome.lock().expect("outcome").as_ref().is_some_and(|outcome| outcome.failed));
    }
}
