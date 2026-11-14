#!/usr/bin/env bash
# Build target/bundle/Crow.app. Running Crow as an app (not from a terminal) makes
# macOS privacy prompts (Desktop, Downloads, Full Disk Access) name Crow itself.
#
#   scripts/bundle.sh             build and sign (see Signing below)
#   scripts/bundle.sh --open      ...then launch it
#   scripts/bundle.sh --install   ...then copy it to /Applications (replacing an older Crow)
#
# The options combine: `--install --open` launches the installed copy.
# Set CROW_INSTALL_DIR to install somewhere else (e.g. ~/Applications).
#
#
# Signing: with a code-signing certificate named "Crow Dev" in the keychain, every build
# is signed with it, so macOS keeps Crow's privacy permissions (Full Disk Access, folder
# prompts) from one build to the next. Without it the build is signed ad hoc, and macOS
# asks again after every build. Create it once in Keychain Access: Certificate Assistant >
# Create a Certificate, name "Crow Dev", type "Code Signing" (it needn't be trusted).
# Set CROW_SIGN_IDENTITY to use another identity (e.g. a Developer ID for distribution).
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

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Crow</string>
  <key>CFBundleDisplayName</key><string>Crow</string>
  <key>CFBundleIdentifier</key><string>io.github.paraversal.crow</string>
  <key>CFBundleExecutable</key><string>crow</string>
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
echo "built $app ($version), signed $signer"

if $install; then
  dest_dir="${CROW_INSTALL_DIR:-/Applications}"
  mkdir -p "$dest_dir"
  # A running Crow keeps the old binary until it quits; quit it so the next launch is new.
  osascript -e 'if application id "io.github.paraversal.crow" is running then tell application id "io.github.paraversal.crow" to quit' >/dev/null 2>&1 || true
  rm -rf "$dest_dir/Crow.app"
  ditto "$app" "$dest_dir/Crow.app"
  app="$dest_dir/Crow.app"
  echo "installed $app"
fi

if $open_after; then open "$app"; fi
exit 0
