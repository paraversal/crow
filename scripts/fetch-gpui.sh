#!/usr/bin/env bash
# Fetch Zed's GPUI (with AccessKit accessibility) at a pinned commit into vendor/zed.
# Only the ~22 crates GPUI needs are checked out (sparse, blobless, depth 1),
# instead of cloning all of Zed (~550 MB).
set -euo pipefail

ZED_REV="${ZED_REV:-6ceb6fee8738274589b59139df27efc191ec2231}"
CRATES=(
  bench_metrics collections derive_refineable gpui gpui_apple gpui_ios gpui_linux gpui_macos
  gpui_macros gpui_platform gpui_shared_string gpui_util gpui_web gpui_wgpu gpui_windows
  http_client refineable scheduler sum_tree zlog ztracing ztracing_macro
)

cd "$(dirname "$0")/.."
dest=vendor/zed
if [ ! -d "$dest/.git" ]; then
  git init -q "$dest"
  git -C "$dest" remote add origin https://github.com/zed-industries/zed.git
  git -C "$dest" config extensions.partialClone origin
  git -C "$dest" sparse-checkout init --cone
fi
git -C "$dest" sparse-checkout set "${CRATES[@]/#/crates/}"
git -C "$dest" fetch -q --depth 1 --filter=blob:none origin "$ZED_REV"

# Check whether $dest is already at FETCH_HEAD with every patch applied in order,
# without touching file mtimes (preserving Cargo and CI rust-cache fingerprints).
already_patched() {
  [ "$(git -C "$dest" rev-parse HEAD 2>/dev/null || true)" = "$(git -C "$dest" rev-parse FETCH_HEAD)" ] || return 1
  local tmp_index
  tmp_index=$(mktemp "${TMPDIR:-/tmp}/crow-gpui-index.XXXXXX") || return 1
  cp "$dest/.git/index" "$tmp_index"
  for patch in patches/gpui-*.patch; do
    [ -e "$patch" ] || continue
    if ! GIT_INDEX_FILE="$tmp_index" git -C "$dest" apply --cached "$PWD/$patch" 2>/dev/null; then
      rm -f "$tmp_index"
      return 1
    fi
  done
  local status=0
  GIT_INDEX_FILE="$tmp_index" git -C "$dest" diff --quiet || status=$?
  rm -f "$tmp_index"
  return "$status"
}

# Crow's small fixes to GPUI, kept as patches so a fresh fetch (and CI) has them.
if ! already_patched; then
  git -C "$dest" checkout -q -f --detach FETCH_HEAD
  for patch in patches/gpui-*.patch; do
    [ -e "$patch" ] || continue
    git -C "$dest" apply "$PWD/$patch"
  done
fi
echo "GPUI at $(git -C "$dest" rev-parse --short HEAD) in $dest"
