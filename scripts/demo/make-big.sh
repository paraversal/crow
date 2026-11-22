#!/usr/bin/env bash
# The big folders the speed clips use (scripts/shots/site.sh), outside the demo home so
# its own shots stay small:
#   <dir>/100000 files   empty files, from crow --make-fixture
#   <dir>/crates         a copy-on-write clone of this Mac's Cargo source cache
#                        (~/.cargo/registry/src: public crate sources, tens of
#                        thousands of real files) for content search
#
#   scripts/demo/make-big.sh /Users/Shared/Demo-big
#
# Needs a built bundle (scripts/bundle.sh) and an APFS volume for the clone.
set -euo pipefail
dir=${1:?usage: make-big.sh <dir>}
root=$(cd "$(dirname "$0")/../.." && pwd)
app="$root/target/bundle/Crow.app/Contents/MacOS/crow"
[ -x "$app" ] || { echo "make-big.sh: build the bundle first (scripts/bundle.sh)" >&2; exit 1; }

mkdir -p "$dir"
[ -d "$dir/100000 files" ] || "$app" --make-fixture "$dir/100000 files" 100000
if [ ! -d "$dir/crates" ]; then
  src=$(find "$HOME/.cargo/registry/src" -mindepth 1 -maxdepth 1 -type d | head -1)
  [ -n "$src" ] || { echo "make-big.sh: no Cargo sources in ~/.cargo/registry/src" >&2; exit 1; }
  cp -cR "$src" "$dir/crates"
fi
echo "big folders: $dir"
