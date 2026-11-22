# Contributing to Crow

Thanks for taking a look. Crow is a Finder-style file explorer for macOS in Rust on GPUI. [docs/architecture.md](docs/architecture.md) is a short map of the code; [docs/features.md](docs/features.md) lists what exists.

## Build

```bash
./scripts/fetch-gpui.sh          # once: Zed's GPUI at a pinned commit, into vendor/, with patches applied
cargo run --release -p crow-app  # opens your home folder
./scripts/bundle.sh --open       # builds and opens target/bundle/Crow.app
```

Use the bundle for hand tests: macOS privacy prompts then name Crow, not your terminal.

## Before every commit

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs the same on macOS. Run the 100k-file bench (`cargo run --release -p crow-app -- --bench target/fixture-100k`, see [docs/testing.md](docs/testing.md)) when a change touches listing, scrolling or first paint, and compare with `main`.

## Rules of the house
- No disk I/O on the main thread; every file operation goes through `crow_fs::ops` and nothing is overwritten in place.
- Every new keyboard behaviour gets a GPUI keystroke test; every interactive element has a role and an aria label.
- Style with theme roles only, never raw palette slots.
- Config files document their defaults, reload live and keep user comments.
- A change that adds or removes a feature updates [docs/features.md](docs/features.md); a keymap change regenerates [docs/keys.md](docs/keys.md) (`CROW_UPDATE_DOCS=1 cargo test -p crow-app keys_doc`); a config change regenerates [docs/configuration.md](docs/configuration.md) (`CROW_UPDATE_DOCS=1 cargo test -p crow-core --test docs`).

## Plans, fuzzing and agents
Larger work follows a plan document with a progress file; [AGENTS.md](AGENTS.md) describes the workflow (it is written for coding agents but reads fine for people) and the fuzz budget: long fuzz runs need a maintainer's OK first, and `cargo test --workspace` already runs a short fuzz smoke.

## Pull requests
One branch per piece of work, squash-merged to `main` after CI is green and a hand test. Describe what changed and how you checked it. By contributing you agree that your work is released under the project's GPL-3.0-or-later licence.
