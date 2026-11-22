#!/usr/bin/env bash
# Build target/bundle/Crow.app. Running Crow as an app (not from a terminal) makes
# macOS privacy prompts (Desktop, Downloads, Full Disk Access) name Crow itself.
#
#   scripts/bundle.sh             build and sign (see Signing below)
#   scripts/bundle.sh --open      ...then launch it
#   scripts/bundle.sh --install   ...then copy it to /Applications (replacing an older Crow)
#                                 and install the `crow` command for the terminal
#   scripts/bundle.sh --release   what scripts/release.sh ships: always signed ad hoc (the
#                                 local "Crow Dev" identity means nothing on other Macs)
#
# CROW_BINARY=path uses that prebuilt binary (release.sh passes its universal one) instead
# of running cargo.
#
# Version: [workspace.package] in Cargo.toml is the one source. The bundle's
# CFBundleShortVersionString is that version; CFBundleVersion is the commit count, so every
# build is newer than the last for macOS; the short commit hash goes in the About credits.
# The options combine: `--install --open` launches the installed copy.
# Set CROW_INSTALL_DIR to install the app somewhere else (e.g. ~/Applications);
# set CROW_BIN_DIR to put the `crow` command elsewhere (default ~/.local/bin).
#
#
# Signing: with a code-signing certificate named "Crow Dev" in the keychain, every build
# is signed with it, so macOS keeps Crow's privacy permissions (Full Disk Access, folder
# prompts) from one build to the next. Without it the build is signed ad hoc, and macOS
# asks again after every build. Create it once in Keychain Access: Certificate Assistant >
# Create a Certificate, name "Crow Dev", type "Code Signing" (it needn't be trusted).
# Set CROW_SIGN_IDENTITY to use another identity (e.g. a Developer ID for distribution).
#
# Icon: assets/app-icon.png (square, any size; full-bleed art gets the Dock's margin added). Replace that file and bundle again; the
# app embeds the same PNG for the Dock, and scripts/site.sh makes the website's sizes.
set -euo pipefail
cd "$(dirname "$0")/.."

open_after=false
install=false
release=false
for arg in "$@"; do
  case "$arg" in
    --open) open_after=true ;;
    --install) install=true ;;
    --release) release=true ;;
    *) echo "unknown option: $arg (use --open, --install, --release)" >&2; exit 2 ;;
  esac
done

[ -d vendor/zed/.git ] || ./scripts/fetch-gpui.sh

pkgid=$(cargo pkgid -p crow-app)
version=${pkgid##*[@#]}
build=$(git rev-list --count HEAD 2>/dev/null || echo 0)
commit=$(git rev-parse --short=7 HEAD 2>/dev/null || echo unknown)
export CROW_BUILD="$build" CROW_COMMIT="$commit"

binary="${CROW_BINARY:-}"
if [ -z "$binary" ]; then
  cargo build --release -p crow-app
  binary=target/release/crow
fi

app=target/bundle/Crow.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/crow"
cp crates/crow-app/assets/icons/LICENSE "$app/Contents/Resources/Lucide-LICENSE.txt"
cp LICENSE "$app/Contents/Resources/LICENSE.txt"
# The terminal launcher; the Homebrew cask links it onto PATH as `crow`.
install -m 755 scripts/crow "$app/Contents/Resources/crow"
cp THIRD-PARTY-NOTICES.md "$app/Contents/Resources/THIRD-PARTY-NOTICES.md"
# The standard About panel shows version and build from Info.plist and this file below them.
cat > "$app/Contents/Resources/Credits.html" <<CREDITS
<html><body style="font-family: -apple-system; font-size: 11px; text-align: center;">
<p>Commit ${commit}<br>Free software under the GNU GPL, version 3 or later.<br>Help &gt; Acknowledgements lists the third-party licences.</p>
</body></html>
CREDITS

icon_src=assets/app-icon.png
iconset=$(mktemp -d)/AppIcon.iconset
mkdir -p "$iconset"
# The Dock draws icons inside a template (artwork ~80% of the canvas); pad-icon.swift adds
# that margin to full-bleed art and leaves art that already has one alone.
icon_png=$(dirname "$iconset")/padded.png
swift scripts/pad-icon.swift "$icon_src" "$icon_png"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$icon_png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) "$icon_png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"
rm -rf "$(dirname "$iconset")"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Crow</string>
  <key>CFBundleDisplayName</key><string>Crow</string>
  <key>CFBundleIdentifier</key><string>io.github.paraversal.crow</string>
  <key>CFBundleExecutable</key><string>crow</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${build}</string>
  <key>NSHumanReadableCopyright</key><string>Copyright © Crow contributors. GPL-3.0-or-later.</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSDesktopFolderUsageDescription</key><string>Crow shows and manages the files on your Desktop.</string>
  <key>NSDocumentsFolderUsageDescription</key><string>Crow shows and manages the files in Documents.</string>
  <key>NSDownloadsFolderUsageDescription</key><string>Crow shows and manages the files in Downloads.</string>
  <key>NSRemovableVolumesUsageDescription</key><string>Crow shows and manages files on external drives.</string>
  <key>NSAppleEventsUsageDescription</key><string>Crow asks Finder to empty the Trash when macOS keeps Crow from reading it.</string>
  <key>NSNetworkVolumesUsageDescription</key><string>Crow shows and manages files on network shares.</string>
</dict>
</plist>
PLIST

identity="${CROW_SIGN_IDENTITY:-}"
if $release; then
  identity=-
elif [ -z "$identity" ] && security find-identity -p codesigning 2>/dev/null | grep -q '"Crow Dev"'; then
  identity="Crow Dev"
fi
codesign --force --sign "${identity:--}" --timestamp=none "$app"
[ "${identity:--}" = "-" ] && signer="ad hoc" || signer="$identity"
echo "built $app ($version, build $build, $commit), signed $signer"

if $install; then
  dest_dir="${CROW_INSTALL_DIR:-/Applications}"
  mkdir -p "$dest_dir"
  # A running Crow keeps the old binary until it quits; quit it so the next launch is new.
  osascript -e 'if application id "io.github.paraversal.crow" is running then tell application id "io.github.paraversal.crow" to quit' >/dev/null 2>&1 || true
  rm -rf "$dest_dir/Crow.app"
  ditto "$app" "$dest_dir/Crow.app"
  app="$dest_dir/Crow.app"
  echo "installed $app"

  # The command is a small shell script, not a copy of the app: it runs `open -a`,
  # so reinstalling or rebuilding Crow at the same path needs no update.
  bin_dir="${CROW_BIN_DIR:-$HOME/.local/bin}"
  mkdir -p "$bin_dir" 2>/dev/null || true
  if [ ! -d "$bin_dir" ]; then
    echo "crow command: $bin_dir does not exist; set CROW_BIN_DIR" >&2
  elif [ -w "$bin_dir" ]; then
    install -m 755 scripts/crow "$bin_dir/crow"
    echo "installed $bin_dir/crow"
    case ":$PATH:" in
      *":$bin_dir:"*) ;;
      *) echo "note: add $bin_dir to PATH to run \`crow\`" >&2 ;;
    esac
    if [ "$app" != "/Applications/Crow.app" ]; then
      echo "note: $bin_dir/crow points at /Applications/Crow.app; set CROW_APP=$app" >&2
    fi
  else
    echo "crow command: cannot write $bin_dir; set CROW_BIN_DIR to a writable directory" >&2
  fi
fi

if $open_after; then open "$app"; fi
exit 0
