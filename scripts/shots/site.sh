#!/usr/bin/env bash
# Retake every screenshot the website and README use, then write them to docs/media as
# WebP. Run after a UI change; the demo home must exist first:
#
#   scripts/demo/make-demo.sh /Users/Shared/Demo
#   scripts/demo/make-personas.sh /Users/Shared
#   scripts/bundle.sh
#   scripts/shots/site.sh
#
# The demo home lives in /Users/Shared so the path bar reads Macintosh HD › Users ›
# Shared › Demo rather than a checkout path. Needs cwebp (brew install webp) and the
# permissions scripts/shot.sh lists. Don't touch the Mac while it runs (~2 minutes).
set -euo pipefail
cd "$(dirname "$0")/../.."

D=${CROW_DEMO_HOME:-/Users/Shared/Demo}
PARENT=${CROW_PERSONAS_PARENT:-/Users/Shared}
[ -d "$PARENT/Demo-photographer" ] || { echo "no persona homes in $PARENT; run scripts/demo/make-personas.sh $PARENT" >&2; exit 1; }
[ -d "$D/Pictures/Trips" ] || { echo "no demo home at $D; run scripts/demo/make-demo.sh $D" >&2; exit 1; }
command -v cwebp >/dev/null || { echo "cwebp missing: brew install webp" >&2; exit 1; }

export CROW_SHOT_HOME=$D
preview_on='show_preview = true
show_hidden_files = false'
preview_off='show_preview = false
show_hidden_files = false'
down9='key:down|key:down|key:down|key:down|key:down|key:down|key:down|key:down|key:down'

# shot <webp name> <shot.sh args...>; shot.sh writes target/shots/<state>-<theme>.png.
shot() {
  local name=$1 state=$2 theme=$4
  ./scripts/shot.sh "${@:2}" >/dev/null
  cwebp -quiet -q 86 -alpha_q 100 "target/shots/$state-$theme.png" -o "docs/media/$name.webp"
  echo "docs/media/$name.webp"
}

export CROW_SHOT_SETTINGS=$preview_on
# The hero is the photographer's setup: groups, emoji, the Lucide pack, big thumbnails.
PH=$PARENT/Demo-photographer
for theme in finder-light finder-dark; do
  CROW_SHOT_HOME=$PH CROW_SHOT_PERSONA=photographer \
    shot "hero-${theme#finder-}" hero "$PH/Pictures/Shoots/2026/2026-07 Faroe Islands/Selects" "$theme" finder \
    'wait:2.5|cmd:1|key:down|key:right|key:right|wait:3'
done
shot view-icons view-icons "$D/Pictures/Trips" finder-light finder 'wait:2.5|cmd:1|key:down|key:right|key:right|key:right|wait:3'
shot view-list view-list "$D/Pictures/Trips" finder-light finder "wait:2.5|cmd:2|$down9|wait:3"
shot view-columns view-columns "$D/Pictures/Trips" finder-light finder "wait:2.5|cmd:3|$down9|wait:3"
CROW_SHOT_SETTINGS=$preview_off \
  shot view-gallery view-gallery "$D/Pictures/Trips" finder-light finder "wait:2.5|cmd:4|$down9|wait:3"
shot palette palette "$D/Projects/tidepool" finder-light finder 'wait:2.5|cmd:k|wait:0.6|text:pro|wait:1.5'
shot search search "$D" finder-light finder 'wait:2.5|cmd:f|wait:0.5|text:raven|wait:3'
shot git-list list "$D/Projects/tidepool/src" finder-light finder 'wait:2.5|cmd:2|wait:0.5|key:down|key:down|wait:2.5'

# "Make it yours": four people, four setups (scripts/demo/personas/).
persona() { # <persona> <webp> <home folder> <start under home> <theme> <keys>
  CROW_SHOT_HOME=$PARENT/Demo-$3 CROW_SHOT_PERSONA=$1 shot "$2" "$2" "$PARENT/Demo-$3/$4" "$5" finder "$6"
}
persona developer persona-developer developer "Projects/work/acme-api/src" catppuccin-mocha 'wait:2.5|cmd:2|key:down|key:down|key:down|key:down|key:down|key:down|wait:3'
CROW_SHOT_SETTINGS=$preview_off \
  persona photographer persona-photographer photographer "Pictures/Shoots/2026/2026-03 Lofoten/Selects" catppuccin-latte 'wait:2.5|cmd:4|key:down|wait:3'
persona writer persona-writer writer "Manuscripts/The Lantern Year/Chapters" gruvbox-light 'wait:2.5|cmd:2|key:down|key:down|key:down|key:down|wait:3'
persona producer persona-producer producer "Sample Library/Drums" solarized-dark 'wait:2.5|cmd:3|key:down|key:right|key:down|wait:3'
