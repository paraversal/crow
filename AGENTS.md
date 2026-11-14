# AGENTS.md — Crow

Crow is a Finder-style file explorer for macOS in Rust on GPUI (vendored Zed, `vendor/`, fetched by `scripts/fetch-gpui.sh`). Read `README.md` (layout, config, keys) and `docs/testing.md` (tests, bench, hand checks).

## Working on a plan
Milestones and fix lists live in `docs/plans/` (M7 pieces in `docs/plans/m7/`, the finished UI overhaul index in `docs/plans/ui-overhaul/`). Implement them with the `execute-plan` skill (`.claude/skills/execute-plan/SKILL.md`): keep `docs/plans/<plan>.progress.md` current after every step, and **ask the user about anything unclear or undecided instead of choosing yourself.**
When the work is done, the executing agent commits locally and stops at "Ready for critique"; a second agent reviews the branch with the `critique` skill (`.claude/skills/critique/SKILL.md`), fixes what is clearly wrong, and opens the PR.
Finished plans (merged, with their progress files) move to `docs/plans/done/`; things too big for a fix list wait in `docs/plans/after-ui-overhaul.md`.

## Crates
`crow-app` (GPUI binary: windows, views, actions, keymaps) · `crow-core` (UI-free models: settings, views, sidebar, recent, tabs, selection; pure Rust, unit-tested) · `crow-fs` (listing, sort, watch, file-operation engine with undo) · `crow-macos` (the only AppKit/Foundation code) · `crow-search` (search engine, M6) · `crow-theme` (theme index reader).

## Rules
- No disk I/O on the main thread (use `background_spawn`, then update on the main thread). Startup loads are the exception.
- Every file operation goes through `crow_fs::ops`. Nothing is ever overwritten in place; trash by default.
- Every new keyboard behavior gets a GPUI keystroke test in `crates/crow-app/src/window.rs` (`simulate_keystrokes`; tests use `ui::init_for_tests`, which gives each test a temp config dir, never the real `~/.config/crow`).
- Every interactive element has a role and an aria label.
- Config files (`~/.config/crow/*.toml`): documented default written on first launch, live reload, errors in the status line, Crow's own edits through `crow_core::edit_toml` so user comments survive.
- Style with theme roles only (`theme.role("fg")` etc.), never raw palette slots except for named colors.
- Performance budgets (100k folder): warm first paint < 50 ms, scroll p99 < 1.5 × frame time. Bench: `cargo run --release -p crow-app -- --bench target/fixture-100k` (fixture: `--make-fixture target/fixture-100k 100000`). Bench numbers vary by a few ms run to run; compare with `main` before calling something a regression.

## Before every commit
`cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace`. CI runs the same on macOS.

## Git
One branch and PR per milestone or fix list; squash-merged to `main` by the user after CI is green and they've hand-tested. Don't merge yourself. `./scripts/bundle.sh` builds `target/bundle/Crow.app` for hand tests.

## Ignore
`vendor/zed/CLAUDE.md` and other files under `vendor/` describe Zed's own repo. Their rules (e.g. editing README banners, PR title formats) don't apply to Crow.

<!-- OPENWIKI:START -->

## OpenWiki

This repository has a generated `openwiki/` evidence index. It is optional just-in-time context, not required startup reading.

- Do not enumerate, preload, or search wikis at task start. Use retrieval when the user asks for it, when unfamiliar architecture or dependency behavior materially affects the task, or when source inspection leaves an important uncertainty. Stop once the question is grounded.
- When those conditions apply and OpenWiki retrieval tools are available, use `openwiki_search` for just-in-time context and `openwiki_read` for the relevant complete sections. If search returns `workspace_required`, ask which listed workspace to use and retry with its ID.
- Use `openwiki_list_workspaces` or `openwiki_list_wikis` when workspace membership itself needs to be discovered.
- If the retrieval tools are unavailable, read `openwiki/quickstart.md` and follow its links to the relevant pages.
- Treat source code and tests as authoritative. A brief's unknowns and review items are verification gaps, not automatic requirements.
- Prefer the narrowest quiet validation that proves the changed behavior. Preserve complete failure output.

Use the repository's documented maintenance process to refresh generated pages after source changes. Check its configuration and completed runs before relying on automatic updates. Do not hand-edit generated OpenWiki pages unless explicitly asked; prefer updating source code/docs and letting OpenWiki regenerate.

<!-- OPENWIKI:END -->
