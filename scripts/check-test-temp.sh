#!/usr/bin/env bash
# Run the workspace tests with TMPDIR pointing at a private parent, then fail if
# any Crow test fixture survived the run.
#
# The palette tests keep their fixtures under /tmp on purpose: they score whole
# paths, and a random $TMPDIR prefix can make a query match an unrelated folder.
# This script therefore inventories the new /tmp entries with Crow's exact
# prefixes too, never sweeping or deleting anything it did not create.
set -uo pipefail

if ! command -v cargo >/dev/null 2>&1; then
    echo "check-test-temp: cargo is not on PATH" >&2
    exit 2
fi

parent=$(mktemp -d "${TMPDIR:-/tmp}/crow-temp-check.XXXXXX") || exit 2
run="$parent/run"
test_tmp="$run/tmp"
mkdir -p "$test_tmp" || exit 2

cleanup() {
    chmod -R u+rwx "$parent" 2>/dev/null || true
    rm -rf "$parent"
}
trap cleanup EXIT INT TERM

# The private parent must start empty; the /tmp baseline records what was
# already there so the report can name only this run's leftovers.
if [ -n "$(find "$test_tmp" -mindepth 1 -print -quit)" ]; then
    echo "check-test-temp: $test_tmp was not empty" >&2
    exit 2
fi
# /tmp is a symlink to /private/tmp on macOS; `find /tmp` does not descend it.
tmp_root=$(cd /tmp && pwd -P) || exit 2
before=$(mktemp "$parent/before.XXXXXX") || exit 2
after=$(mktemp "$parent/after.XXXXXX") || exit 2
find "$tmp_root" -maxdepth 1 \( -name 'crow-*' -o -name '.trash-*' \) -print | sort >"$before"

echo "check-test-temp: TMPDIR=$test_tmp"
echo "check-test-temp: pre-existing /tmp Crow fixtures: $(wc -l <"$before" | tr -d ' ')"

status=0
TMPDIR="$test_tmp" cargo test --workspace "$@" || status=$?

find "$test_tmp" -mindepth 1 -print | sort >"$parent/leftovers"
find "$tmp_root" -maxdepth 1 \( -name 'crow-*' -o -name '.trash-*' \) -print | sort >"$after"
comm -13 "$before" "$after" >"$parent/new-tmp"

leaked=0
if [ -s "$parent/leftovers" ]; then
    leaked=1
    echo "check-test-temp: leftover fixtures under TMPDIR:" >&2
    sed -n '1,80p' "$parent/leftovers" >&2
fi
if [ -s "$parent/new-tmp" ]; then
    leaked=1
    echo "check-test-temp: new Crow fixtures under /tmp:" >&2
    sed -n '1,80p' "$parent/new-tmp" >&2
fi

if [ "$leaked" -eq 1 ]; then
    status=1
else
    echo "check-test-temp: clean — no Crow fixtures left in $test_tmp or /tmp"
fi
exit "$status"
