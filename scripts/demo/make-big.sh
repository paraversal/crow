#!/usr/bin/env bash
# The big folders the speed clips use (scripts/shots/site.sh), outside the demo home so
# its own shots stay small:
#   <dir>/100000 files   mixed types, sizes and dates (scripts/demo/mixed-folder.py)
#   <dir>/crates         a copy-on-write clone of this Mac's Cargo source cache
#                        (~/.cargo/registry/src: public crate sources, tens of
#                        thousands of real files) for content search
#
#   scripts/demo/make-big.sh /Users/Shared/Demo-big
#
# Needs an APFS volume for the clone.
set -euo pipefail
dir=${1:?usage: make-big.sh <dir>}
root=$(cd "$(dirname "$0")/../.." && pwd)

mkdir -p "$dir"
[ -d "$dir/100000 files" ] || python3 "$root/scripts/demo/mixed-folder.py" "$dir/100000 files" 100000
# Its sparse files add up to over a terabyte that a backup could expand.
tmutil addexclusion "$dir" 2>/dev/null || echo "make-big.sh: exclude $dir from backups by hand" >&2
if [ ! -d "$dir/crates" ]; then
  src=$(find "$HOME/.cargo/registry/src" -mindepth 1 -maxdepth 1 -type d | head -1)
  [ -n "$src" ] || { echo "make-big.sh: no Cargo sources in ~/.cargo/registry/src" >&2; exit 1; }
  cp -cR "$src" "$dir/crates"
fi
echo "big folders: $dir"
