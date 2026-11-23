#!/usr/bin/env bash
# Render the website's component shots (docs/media/<name>-light.webp and -dark.webp): single
# Crow components cut out of the real window with fixture data, so a change to a native
# component changes its shot. No demo home, no Screen Recording permission and nothing
# on screen: GPUI renders the window offscreen (`crow --component-shot`, crates/crow-app/src/component_shot.rs).
#
#   scripts/component-shots.sh                  every component, light and dark
#   scripts/component-shots.sh git-rows         only those
#
# Components: git-rows git-sidebar git-diff compare-gallery plugin-surfaces
# Themes: finder-light and finder-dark (built in), like scripts/shots/site.sh.
# Needs cwebp (brew install webp). PNGs stay in target/component-shots for a look.
# The binary is a debug build with the `component-shots` feature (only debug builds
# record the element bounds the crops come from); it is never part of the bundle.
set -euo pipefail
cd "$(dirname "$0")/.."

command -v cwebp >/dev/null || { echo "cwebp missing: brew install webp" >&2; exit 1; }
all=(git-rows git-sidebar git-diff compare-gallery plugin-surfaces)
names=("$@")
[ ${#names[@]} -gt 0 ] || names=("${all[@]}")

cargo build -q -p crow-app --features component-shots
bin=target/debug/crow
mkdir -p target/component-shots

for name in "${names[@]}"; do
  for variant in light dark; do
    png="target/component-shots/$name-$variant.png"
    "$bin" --component-shot "$name" --theme "finder-$variant" --scale 2 --out "$png"
    cwebp -quiet -q 86 -alpha_q 100 "$png" -o "docs/media/$name-$variant.webp"
    echo "docs/media/$name-$variant.webp"
  done
done
