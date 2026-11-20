#!/usr/bin/env bash
# Build target/bundle/Crow.app. Running Crow as an app (not from a terminal) makes
# macOS privacy prompts (Desktop, Downloads, Full Disk Access) name Crow itself.
#
#   scripts/bundle.sh             build and sign (see Signing below)
#   scripts/bundle.sh --open      ...then launch it
#   scripts/bundle.sh --install   ...then copy it to /Applications (replacing an older Crow)
#                                 and install the `crow` command for the terminal
#
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
# Icon: the bundle carries the icon named by `app_icon` in your settings.toml (refined,
# sun, graphite or bone; refined when unset), so Finder and Launchpad match the Dock.
# CROW_APP_ICON overrides it. The PNGs come from crates/crow-app/assets/app-icon.
set -euo pipefail
cd "$(dirname "$0")/.."

open_after=false
install=false
for arg in "$@"; do
  case "$arg" in
    --open) open_after=true ;;
    --install) install=true ;;
    *) echo "unknown option: $arg (use --open, --install)" >&2; exit 2 ;;
  esac
done

[ -d vendor/zed/.git ] || ./scripts/fetch-gpui.sh
cargo build --release -p crow-app

version=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/crow-app/Cargo.toml | head -1)
app=target/bundle/Crow.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/crow "$app/Contents/MacOS/crow"
cp crates/crow-app/assets/icons/LICENSE "$app/Contents/Resources/Lucide-LICENSE.txt"

# The icon: CROW_APP_ICON, else app_icon from settings.toml, else refined.
config_dir="${CROW_CONFIG_DIR:-${XDG_CONFIG_HOME:-$HOME/.config}/crow}"
icon="${CROW_APP_ICON:-}"
if [ -z "$icon" ] && [ -f "$config_dir/settings.toml" ]; then
  icon=$(sed -n 's/^[[:space:]]*app_icon[[:space:]]*=[[:space:]]*"\([a-z]*\)".*/\1/p' "$config_dir/settings.toml" | head -1)
fi
icon="${icon:-refined}"
icon_png="crates/crow-app/assets/app-icon/$icon.png"
if [ ! -f "$icon_png" ]; then
  echo "unknown app icon \"$icon\" (refined, sun, graphite, bone); using refined" >&2
  icon=refined
  icon_png="crates/crow-app/assets/app-icon/refined.png"
fi
iconset=$(mktemp -d)/AppIcon.iconset
mkdir -p "$iconset"
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
  <key>CFBundleVersion</key><string>${version}</string>
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
if [ -z "$identity" ] && security find-identity -p codesigning 2>/dev/null | grep -q '"Crow Dev"'; then
  identity="Crow Dev"
fi
codesign --force --sign "${identity:--}" --timestamp=none "$app"
[ "${identity:--}" = "-" ] && signer="ad hoc" || signer="$identity"
echo "built $app ($version, $icon icon), signed $signer"

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
