#!/usr/bin/env bash
# Build the website into target/site: everything in docs/ except the plans, plus the
# icon sizes made from assets/app-icon.png, so replacing that one PNG updates the
# app, the bundle and the site. GitHub Pages publishes target/site
# (.github/workflows/pages.yml); macOS only (sips).
#
#   scripts/site.sh           build target/site
#   scripts/site.sh --serve   build, then serve it on http://127.0.0.1:8765
set -euo pipefail
cd "$(dirname "$0")/.."

out=target/site
rm -rf "$out"
mkdir -p "$out"
rsync -a --exclude 'plans/' docs/ "$out/"

icon=assets/app-icon.png
sips -z 256 256 "$icon" --out "$out/media/icon-256.png" >/dev/null
sips -z 64 64 "$icon" --out "$out/media/favicon.png" >/dev/null
echo "built $out"

if [ "${1:-}" = "--serve" ]; then
  exec python3 -m http.server 8765 --bind 127.0.0.1 --directory "$out"
fi
