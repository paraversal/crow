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
git -C "$dest" checkout -q --detach FETCH_HEAD
# Crow's small fixes to GPUI, kept as patches so a fresh fetch (and CI) has them.
for patch in patches/gpui-*.patch; do
  [ -e "$patch" ] || continue
  if ! git -C "$dest" apply --reverse --check "$PWD/$patch" 2>/dev/null; then
    git -C "$dest" apply "$PWD/$patch"
  fi
done
echo "GPUI at $(git -C "$dest" rev-parse --short HEAD) in $dest"
