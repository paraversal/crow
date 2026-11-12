#!/usr/bin/env bash
# Build target/bundle/Crow.app. Running Crow as an app (not from a terminal) makes
# macOS privacy prompts (Desktop, Downloads, Full Disk Access) name Crow itself.
#
#   scripts/bundle.sh          build and sign ad hoc
#   scripts/bundle.sh --open   ...then launch it
#
# Set CROW_SIGN_IDENTITY to a Developer ID to sign for distribution instead.
set -euo pipefail
cd "$(dirname "$0")/.."

[ -d vendor/zed/.git ] || ./scripts/fetch-gpui.sh
cargo build --release -p crow-app

version=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/crow-app/Cargo.toml | head -1)
app=target/bundle/Crow.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/crow "$app/Contents/MacOS/crow"

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

codesign --force --sign "${CROW_SIGN_IDENTITY:--}" --timestamp=none "$app"
echo "built $app ($version)"
[ "${1:-}" = "--open" ] && open "$app"
exit 0
