#!/usr/bin/env bash
# Build target/release-dmg/Crow-<version>.dmg: a universal (Apple Silicon + Intel), ad hoc
# signed app in a compressed disk image with an /Applications shortcut, its SHA-256, and a
# smoke check that the app in the mounted image starts and quits.
#
#   scripts/release.sh            refuses a dirty tree or an existing v<version> tag
#   scripts/release.sh --no-tag   skips the tag check (CI builds, rebuilding a release)
#
# CROW_ARCHS="aarch64 x86_64" picks the architectures (default both; one gives a thin build).
# CROW_TAP_DIR=path to a checkout of the Homebrew tap updates Casks/crow.rb there (version,
# sha256); otherwise the new values are printed to paste. The script never tags, pushes or
# publishes: that is for a person.
set -euo pipefail
cd "$(dirname "$0")/.."

check_tag=true
for arg in "$@"; do
  case "$arg" in
    --no-tag) check_tag=false ;;
    *) echo "unknown option: $arg (use --no-tag)" >&2; exit 2 ;;
  esac
done

version=$(cargo pkgid -p crow-app); version=${version##*[@#]}
tag="v$version"

if [ -n "$(git status --porcelain)" ]; then
  echo "refusing to release from a dirty tree:" >&2
  git status --short >&2
  exit 1
fi
if $check_tag && git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "refusing: tag $tag already exists (bump [workspace.package] version, or use --no-tag)" >&2
  exit 1
fi

archs="${CROW_ARCHS:-aarch64 x86_64}"
out=target/release-dmg
rm -rf "$out"
mkdir -p "$out"

# Build and merge. CROW_BUILD/CROW_COMMIT are the same for every slice.
export CROW_BUILD=$(git rev-list --count HEAD) CROW_COMMIT=$(git rev-parse --short=7 HEAD)
slices=()
for arch in $archs; do
  target="$arch-apple-darwin"
  cargo build --release -p crow-app --target "$target" --locked
  slices+=("target/$target/release/crow")
done
binary="$out/crow"
if [ "${#slices[@]}" -eq 1 ]; then
  cp "${slices[0]}" "$binary"
else
  lipo -create "${slices[@]}" -output "$binary"
fi
lipo -info "$binary"

CROW_BINARY="$binary" ./scripts/bundle.sh --release
app=target/bundle/Crow.app

# The image: the app and a shortcut to /Applications.
dmg="$out/Crow-$version.dmg"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
ditto "$app" "$stage/Crow.app"
ln -s /Applications "$stage/Applications"
hdiutil create -quiet -volname "Crow" -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$dmg"

sha=$(shasum -a 256 "$dmg" | awk '{print $1}')
echo "$sha  $(basename "$dmg")" > "$dmg.sha256"
echo "$dmg"
echo "sha256 $sha"

# Smoke check: mount the image, start the app from it with an empty HOME, wait for the
# log's version line, then stop it.
mnt=$(mktemp -d)
home=$(mktemp -d)
cleanup() {
  [ -n "${pid:-}" ] && kill "$pid" 2>/dev/null || true
  hdiutil detach -quiet "$mnt" 2>/dev/null || true
  rm -rf "$stage" "$mnt" "$home"
}
trap cleanup EXIT
hdiutil attach -quiet -nobrowse -readonly -mountpoint "$mnt" "$dmg"
[ -d "$mnt/Crow.app" ] && [ -L "$mnt/Applications" ] || { echo "smoke: the image lacks Crow.app or the Applications link" >&2; exit 1; }
HOME="$home" "$mnt/Crow.app/Contents/MacOS/crow" "$home" >/dev/null 2>&1 &
pid=$!
log="$home/Library/Logs/Crow/crow.log"
for _ in $(seq 1 100); do
  grep -q "Crow $version" "$log" 2>/dev/null && break
  kill -0 "$pid" 2>/dev/null || { echo "smoke: the app exited early" >&2; cat "$log" >&2 2>/dev/null; exit 1; }
  sleep 0.2
done
grep -q "Crow $version" "$log" 2>/dev/null || { echo "smoke: no version line in the log after 20 s" >&2; exit 1; }
kill -TERM "$pid"
wait "$pid" 2>/dev/null || true
pid=
echo "smoke ok: $(grep -m1 "Crow $version" "$log" | sed 's/^[^C]*//')"

# The cask: update a local tap checkout, or print what to paste.
if [ -n "${CROW_TAP_DIR:-}" ] && [ -f "$CROW_TAP_DIR/Casks/crow.rb" ]; then
  sed -i '' -e "s/^  version \".*\"/  version \"$version\"/" -e "s/^  sha256 \".*\"/  sha256 \"$sha\"/" "$CROW_TAP_DIR/Casks/crow.rb"
  echo "updated $CROW_TAP_DIR/Casks/crow.rb (review, commit and push it yourself)"
else
  printf 'cask:\n  version "%s"\n  sha256 "%s"\n' "$version" "$sha"
fi
