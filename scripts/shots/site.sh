#!/usr/bin/env bash
# Retake every screenshot the website and README use, then write them to docs/media as
# WebP. Run after a UI change; the demo home must exist first:
#
#   scripts/demo/make-demo.sh /Users/Shared/Demo
#   scripts/demo/make-personas.sh /Users/Shared
#   scripts/bundle.sh
#   scripts/shots/site.sh               every shot
#   scripts/shots/site.sh search info   only those (the <webp name> before -light/-dark)
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

# shot <webp name> <state> <start dir> <theme> <keys>: shot.sh writes
# target/shots/<state>-<theme>.png, which becomes docs/media/<name>.webp.
shot() {
  local name=$1 state=$2 theme=$4
  ./scripts/shot.sh "${@:2}" >/dev/null
  cwebp -quiet -q 86 -alpha_q 100 "target/shots/$state-$theme.png" -o "docs/media/$name.webp"
  echo "docs/media/$name.webp"
}

# both <webp name> <state> <start dir> <light theme> <dark theme> <keys>: a light and a dark shot,
# <name>-light.webp and <name>-dark.webp, so the page can follow the system appearance.
# CROW_SHOT_KEYMAP=vim takes it with the Vim layer.
only=" $* "
both() {
  [ "$only" = "  " ] || [[ $only == *" $1 "* ]] || return 0
  shot "$1-light" "$2-light" "$3" "$4" "${CROW_SHOT_KEYMAP:-finder}" "$6"
  shot "$1-dark" "$2-dark" "$3" "$5" "${CROW_SHOT_KEYMAP:-finder}" "$6"
}

export CROW_SHOT_SETTINGS=$preview_on
# The hero is the photographer's setup: groups, emoji, the Lucide pack, big thumbnails.
PH=$PARENT/Demo-photographer
CROW_SHOT_HOME=$PH CROW_SHOT_PERSONA=photographer CROW_SHOT_SIDEBAR_STYLE=ledger \
  both hero hero "$PH/Pictures/Shoots/2026/2026-07 Faroe Islands/Selects" finder-light finder-dark \
  'wait:2.5|cmd:1|key:down|key:right|key:right|wait:3'

both view-icons view-icons "$D/Pictures/Trips" finder-light finder-dark 'wait:2.5|cmd:1|key:down|key:right|key:right|key:right|wait:3'
both view-list view-list "$D/Pictures/Trips" finder-light finder-dark "wait:2.5|cmd:2|$down9|wait:3"
both view-columns view-columns "$D/Pictures/Trips" finder-light finder-dark "wait:2.5|cmd:3|$down9|wait:3"
CROW_SHOT_SETTINGS=$preview_off \
  both view-gallery view-gallery "$D/Pictures/Trips" finder-light finder-dark "wait:2.5|cmd:4|$down9|wait:3"
both palette palette "$D/Projects/tidepool" finder-light finder-dark 'wait:2.5|cmd:k|wait:0.6|text:pro|wait:1.5'
both search search "$D" finder-light finder-dark 'wait:2.5|cmd:f|wait:0.5|text:raven|wait:3'
both git-list git-list "$D/Projects/tidepool/src" finder-light finder-dark 'wait:2.5|cmd:2|wait:0.5|key:down|key:down|wait:2.5'

# "Make it yours": four people, four setups (scripts/demo/personas/), each in a light and a dark theme.
persona() { # <persona> <home> <start under home> <light theme> <dark theme> <keys>
  CROW_SHOT_HOME=$PARENT/Demo-$2 CROW_SHOT_PERSONA=$1 \
    both "persona-$1" "persona-$1" "$PARENT/Demo-$2/$3" "$4" "$5" "$6"
}
persona developer developer "Projects/work/acme-api/src" catppuccin-latte catppuccin-mocha \
  'wait:2.5|cmd:2|key:down|key:down|key:down|key:down|key:down|key:down|wait:3'
CROW_SHOT_SETTINGS=$preview_off \
  persona photographer photographer "Pictures/Shoots/2026/2026-03 Lofoten/Selects" rose-pine-dawn rose-pine \
  'wait:2.5|cmd:4|key:down|wait:3'
persona writer writer "Manuscripts/The Lantern Year/Chapters" gruvbox-light gruvbox-dark \
  'wait:2.5|cmd:2|key:down|key:down|key:down|key:down|wait:2.5'
persona producer producer "Sample Library/Drums" solarized-light solarized-dark \
  'wait:2.5|cmd:3|key:down|key:right|key:down|wait:3'

# The manual's task shots, in the Finder themes.
both batch-rename batch-rename "$D/Pictures/Trips" finder-light finder-dark \
  'wait:2.5|cmd:2|key:down|cmd:a|key:f2|wait:1|text:Lake|key:tab|text:Loch|wait:1.5'
# Two photos: the Info tab's totals and shared tags (one item would show its owner, this Mac's user).
both info info "$D/Pictures/Trips" finder-light finder-dark \
  'wait:2.5|cmd:2|key:down|key:down|shift:down|cmd:i|wait:2.5'
both filter filter "$D/Downloads" finder-light finder-dark 'wait:2.5|cmd:2|text:/|wait:0.5|text:pdf|wait:1.5'
# Downloads/Ideas.md pasted over Documents/Notes/Ideas.md: the conflict comes up before anything copies.
both copy-conflict copy-conflict "$D/Downloads" finder-light finder-dark \
  'wait:2.5|cmd:2|text:Ideas|wait:0.5|cmd:c|cmdshift:g|wait:0.8|cmd:a|text:~/Documents/Notes|wait:0.5|key:return|wait:2|cmd:v|wait:2'
# Settings ▸ Keyboard with the Vim layer on: the Vim column of the shortcut list.
CROW_SHOT_KEYMAP=vim CROW_SHOT_OTHER_WINDOW=1 \
  both keys keys "$D/Documents" finder-light finder-dark 'wait:2.5|cmd:,|wait:2|text:keyboard|wait:1.5'
CROW_SHOT_OTHER_WINDOW=1 \
  both settings settings "$D/Documents" finder-light finder-dark 'wait:2.5|cmd:,|wait:2.5'
