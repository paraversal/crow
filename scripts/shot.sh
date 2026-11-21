#!/usr/bin/env bash
# Screenshot Crow's window at a fixed size from a clean config, so the result can
# be compared with the canvas boards and retaken later.
#
#   scripts/shot.sh <state> <start-dir> [theme] [keymap] [keys]
#
# <state> names the output file: target/shots/<state>-<theme>.png. The window is
# 1280x800 (the boards' size), the theme is a theme-index id (never "system", so
# the shot does not follow the Mac's appearance), hints are "always", and the
# state dir is fresh, so Recent holds only what this run visits.
#
# [keys] drives the state from the command line, `|`-separated, each step one of:
#   text:vi          type the text
#   cmd:f            press the letter with Command (cmdshift:a, cmdfn:… likewise)
#   key:escape       return, escape, tab, space, up, down, left, right, delete
#   wait:1.5         sleep that many seconds (a search settling, a tip appearing)
# CROW_SHOT_HOME=<dir> runs Crow with HOME set to <dir> (scripts/demo/make-demo.sh builds
# one), so the path bar reads ~/... and the sidebar lists that home's folders.
# Tags and Locations come from this Mac (Spotlight, mounted volumes), so a demo-home
# shot hides them; its Projects folder is the Git projects root.
# CROW_SHOT_CUSTOM=1 (with CROW_SHOT_HOME) dresses the window up: emoji and colours in
# the sidebar, a second group, and the demo icon pack (scripts/demo/icon-pack).
# CROW_SHOT_SIDEBAR sets the sidebar width in points (default 300: Projects rows fit).
# CROW_SHOT_SETTINGS adds lines to settings.toml (e.g. 'show_preview = true').
# Without [keys] the script waits for Return: set up the state by hand, then
# press Return and it captures. Driving keys needs Accessibility permission for
# the terminal. Needs Screen Recording permission; the script never changes
# privacy settings.
set -euo pipefail

if [ $# -lt 2 ]; then
  sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
  exit 2
fi
state=$1
start_dir=$2
theme=${3:-firebox}
keymap=${4:-finder}
keys=${5:-}

root=$(cd "$(dirname "$0")/.." && pwd)
app="$root/target/bundle/Crow.app/Contents/MacOS/crow"
if [ ! -x "$app" ]; then
  echo "shot.sh: $app is missing; run ./scripts/bundle.sh first" >&2
  exit 1
fi
if [ ! -d "$start_dir" ]; then
  echo "shot.sh: $start_dir is not a folder" >&2
  exit 1
fi

work=$(mktemp -d /tmp/crow-shot.XXXXXX)
mkdir -p "$work/config" "$work/state" "$root/target/shots"
# Past the first-launch sheet: a shot captures the window, not onboarding.
printf '{"tips_choice": true}\n' >"$work/state/onboarding.json"
cat >"$work/config/settings.toml" <<EOF
theme = "$theme"
keymap = "$keymap"
hints = "always"
motion = "off"
${CROW_SHOT_SETTINGS:-}
$( [ -n "${CROW_SHOT_CUSTOM:-}" ] && [ -n "${CROW_SHOT_HOME:-}" ] && echo 'icon_pack = "linework"' )
[window]
width = 1280
height = 800
sidebar_width = ${CROW_SHOT_SIDEBAR:-300}
EOF
# Column view for every state: the boards are column-view windows.
cat >"$work/config/views.toml" <<'EOF'
[defaults]
view = "columns"
EOF
# Favorites inside the repo, so no private folder names reach a shot.
# CROW_SHOT_ROOT points them at a copy of the repo (a neutral path, used when the
# shots are committed: the window's path bar then shows no home folder).
favs=$(cd "${CROW_SHOT_ROOT:-$root}" && pwd -P)
if [ -n "${CROW_SHOT_HOME:-}" ]; then
  shot_home=$(cd "$CROW_SHOT_HOME" && pwd -P)
  fav_items=""
  for d in Desktop Documents Downloads Pictures Projects; do
    [ -d "$shot_home/$d" ] && fav_items="$fav_items  { path = \"$shot_home/$d\" },
"
  done
  if [ -n "${CROW_SHOT_CUSTOM:-}" ]; then
    # "What your Crow could look like": emoji and colours on favorites, a second
    # group, and the demo icon pack.
    fav_items="  { path = \"$shot_home/Desktop\", icon = \"🖥️\" },
  { path = \"$shot_home/Documents\", icon = \"📚\", color = \"yellow\" },
  { path = \"$shot_home/Downloads\", icon = \"📥\", color = \"green\" },
  { path = \"$shot_home/Pictures\", icon = \"🌄\", color = \"purple\" },
  { path = \"$shot_home/Projects\", icon = \"🛠️\", color = \"orange\" },
]

[[group]]
name = \"Trips\"
items = [
  { path = \"$shot_home/Pictures/Trips\", name = \"Photos\", icon = \"📷\", color = \"pink\" },
  { path = \"$shot_home/Documents/Travel\", name = \"Plans\", icon = \"🧭\", color = \"cyan\" },
"
    mkdir -p "$work/config/icon-packs"
    cp -R "$root/scripts/demo/icon-pack" "$work/config/icon-packs/linework"
  fi
  shot_locations=false
  shot_sections="[section.\"tags\"]
show = false

[projects]
roots = [\"$shot_home/Projects\"]"
else
  shot_home=$HOME
  fav_items="  { path = \"$favs\" },
  { path = \"$favs/crates\" },
  { path = \"$favs/docs\" },
  { path = \"$favs/scripts\" },
"
fi
cat >"$work/config/sidebar.toml" <<EOF
[[group]]
name = "Favorites"
items = [
$fav_items]

[recent]
show = true
count = 6

[locations]
show = ${shot_locations:-true}
${shot_sections:-}
EOF

# The bundled Git package runs only with a grant for its exact digest; a demo-home
# shot copies just that record from your config, so badges and Projects show.
if [ -n "${CROW_SHOT_HOME:-}" ] && [ -f "$HOME/.config/crow/plugins.toml" ]; then
  awk '/^\[/{keep = ($0 == "[plugin.git]")} keep && !/^#/' "$HOME/.config/crow/plugins.toml" >"$work/config/plugins.toml"
fi

# No zoxide on PATH: Recent must not pick up folders this run never visited.
# PATH alone is not enough (recent.rs also probes /opt/homebrew/bin and friends),
# so a stub that fails stands in for a real zoxide.
mkdir -p "$work/bin"
printf '#!/bin/sh\nexit 1\n' >"$work/bin/zoxide"
chmod +x "$work/bin/zoxide"
HOME="$shot_home" CROW_CONFIG_DIR="$work/config" CROW_STATE_DIR="$work/state" PATH="$work/bin:/usr/bin:/bin" \
  "$app" "$(cd "$start_dir" && pwd -P)" >"$work/log" 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; rm -rf "$work"' EXIT

window=""
for _ in $(seq 1 60); do
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "shot.sh: Crow exited; log:" >&2
    cat "$work/log" >&2
    exit 1
  fi
  window=$(swift "$root/scripts/window-id.swift" "$pid" 2>/dev/null || true)
  [ -n "$window" ] && break
  sleep 0.5
done
if [ -z "$window" ]; then
  echo "shot.sh: no window for pid $pid after 30 s" >&2
  exit 1
fi
id=${window%% *}
size=${window##* }
if [ "$size" != "1280x800" ]; then
  echo "shot.sh: window is $size, expected 1280x800 (is another Crow running?)" >&2
fi

echo "Crow pid $pid, window $id ($size), theme $theme, keymap $keymap"

press() {
  case $1 in
    text:*) osascript -e "tell application \"System Events\" to keystroke \"${1#text:}\"" ;;
    key:*)
      case ${1#key:} in
        return) code=36 ;;
        escape) code=53 ;;
        tab) code=48 ;;
        space) code=49 ;;
        delete) code=51 ;;
        up) code=126 ;;
        down) code=125 ;;
        left) code=123 ;;
        right) code=124 ;;
        pageup) code=116 ;;
        pagedown) code=121 ;;
        *)
          echo "shot.sh: unknown key ${1#key:}" >&2
          return 1
          ;;
      esac
      osascript -e "tell application \"System Events\" to key code $code"
      ;;
    cmd:*) osascript -e "tell application \"System Events\" to keystroke \"${1#cmd:}\" using command down" ;;
    cmdshift:*)
      osascript -e "tell application \"System Events\" to keystroke \"${1#cmdshift:}\" using {command down, shift down}"
      ;;
    wait:*) sleep "${1#wait:}" ;;
    *)
      echo "shot.sh: unknown step $1" >&2
      return 1
      ;;
  esac
}

if [ -n "$keys" ]; then
  sleep 0.8
  IFS='|' read -ra steps <<<"$keys"
  for step in "${steps[@]}"; do
    press "$step"
    sleep 0.35
  done
  sleep 0.8
else
  echo "Set up the '$state' state in the window, then press Return to capture."
  # A piped or closed stdin is an immediate Return (scripted runs).
  read -r _ || true
fi

out="$root/target/shots/$state-$theme.png"
if ! screencapture -x -o -l "$id" "$out"; then
  echo "shot.sh: screencapture failed; grant the terminal Screen Recording permission" >&2
  exit 1
fi
echo "wrote $out"
