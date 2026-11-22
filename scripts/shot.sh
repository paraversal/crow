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
#   key:escape       return, escape, tab, space, up, down, left, right, delete, f2, home, end
#   shift:down       the same keys with Shift (extend a selection)
#   hold:down:40     press the key 40 times, 50 a second (a held key, for scrolling)
#   ctrlcmd:s        Control-Command with that key (ctrlcmd:s hides the sidebar)
#   wait:1.5         sleep that many seconds (a search settling, a tip appearing)
# CROW_SHOT_HOME=<dir> runs Crow with HOME set to <dir> (scripts/demo/make-demo.sh builds
# one), so the path bar reads ~/... and the sidebar lists that home's folders.
# Tags come from this Mac's Spotlight, so a demo-home shot hides them; Locations (this
# Mac's volumes) stay unless CROW_SHOT_LOCATIONS=false. Its Projects folder is the Git
# projects root.
# CROW_SHOT_CUSTOM=1 (with CROW_SHOT_HOME) dresses the window up: emoji and colours in
# the sidebar, a second group, and the demo icon pack (scripts/demo/icon-pack).
# CROW_SHOT_PERSONA=<id> (with CROW_SHOT_HOME, a make-personas.sh home) uses that persona's
# sidebar groups, settings and the Lucide icon pack instead of the generic demo setup.
# CROW_SHOT_SIDEBAR_STYLE overrides the persona's sidebar_style.
# CROW_SHOT_SIDEBAR sets the sidebar width in points (default 300: Projects rows fit).
# CROW_SHOT_VIEWS adds lines to views.toml's [defaults] (e.g. 'columns = ["modified", "git.lines-changed"]').
# CROW_SHOT_SETTINGS adds lines to settings.toml (e.g. 'show_preview = true').
# CROW_SHOT_OTHER_WINDOW=1 captures the window the keys opened (Settings) instead of
# the browser window.
# CROW_SHOT_RECORD=1 records the window while the keys run instead of a screenshot:
# target/shots/<state>-<theme>.mov, at 60 fps, without the pointer
# (scripts/record-window.swift). Motion stays as configured (Full by default).
# Without [keys] it records CROW_SHOT_RECORD_SECONDS (default 3) instead of waiting.
# CROW_SHOT_ARGS adds arguments before the folder, e.g. --bench (Crow's own 100k
# scroll, which quits after 600 frames: keep the recording shorter than that).
# Without [keys] the script waits for Return: set up the state by hand, then
# press Return and it captures. Crow is brought to the front once before the keys; before
# every key the script checks it is still frontmost and exits if not (tab out to cancel).
# Driving keys needs Accessibility permission for
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
$( [ -z "${CROW_SHOT_CUSTOM:-}" ] && [ -z "${CROW_SHOT_PERSONA:-}" ] && [ -n "${CROW_SHOT_HOME:-}" ] && echo 'icon_pack = "lucide"' )
$( [ -n "${CROW_SHOT_PERSONA:-}" ] && sed "s|^sidebar_style = .*|sidebar_style = \"${CROW_SHOT_SIDEBAR_STYLE:-}\"|;" "$(dirname "$0")/demo/personas/$CROW_SHOT_PERSONA/settings.toml" | { if [ -n "${CROW_SHOT_SIDEBAR_STYLE:-}" ]; then cat; else cat "$(dirname "$0")/demo/personas/$CROW_SHOT_PERSONA/settings.toml"; fi; } )
[window]
width = 1280
height = 800
sidebar_width = ${CROW_SHOT_SIDEBAR:-300}
EOF
# Column view for every state unless CROW_SHOT_VIEW says otherwise: the boards are
# column-view windows.
cat >"$work/config/views.toml" <<EOF
[defaults]
view = "${CROW_SHOT_VIEW:-columns}"
${CROW_SHOT_VIEWS:-}
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
  # The plain demo home: Favorites keep Crow's outline icons, two more groups of folders
  # that exist in it, and Lucide icons for the individual projects (only those folders are
  # in the pack, so every other icon stays the base icon).
  extra_groups=""
  if [ -z "${CROW_SHOT_CUSTOM:-}" ] && [ -z "${CROW_SHOT_PERSONA:-}" ]; then
    group() { # <heading> <path>...
      local heading=$1 items="" rel
      shift
      for rel in "$@"; do
        [ -d "$shot_home/$rel" ] && items="$items  { path = \"$shot_home/$rel\" },
"
      done
      [ -n "$items" ] && extra_groups="$extra_groups
[[group]]
name = \"$heading\"
items = [
$items]
"
    }
    group Paperwork "Documents/Taxes 2025" Documents/Invoices Documents/Travel
    group Media Pictures/Trips Music
    mkdir -p "$work/config/icon-packs"
    LUCIDE_ONLY=projects "$root/scripts/demo/make-lucide-pack.sh" "$work/config/icon-packs/lucide" >/dev/null
  fi
  shot_locations=${CROW_SHOT_LOCATIONS:-true}
  shot_sections="[section.\"tags\"]
show = false

[sections]
order = [\"recent\", \"crow.projects\", \"locations\"]

[projects]
root = \"$shot_home/Projects\""
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
${extra_groups:-}
[recent]
show = true
count = 6

[locations]
show = ${shot_locations:-true}
${shot_sections:-}
EOF

# A persona (scripts/demo/personas/<id>/) brings its own sidebar groups and the Lucide
# icon pack; its home comes from scripts/demo/make-personas.sh.
if [ -n "${CROW_SHOT_PERSONA:-}" ]; then
  pdir="$root/scripts/demo/personas/$CROW_SHOT_PERSONA"
  sed "s|@HOME@|$shot_home|g" "$pdir/sidebar.toml" >"$work/config/sidebar.toml"
  [ -f "$pdir/views.toml" ] && cp "$pdir/views.toml" "$work/config/views.toml"
  mkdir -p "$work/config/icon-packs"
  "$root/scripts/demo/make-lucide-pack.sh" "$work/config/icon-packs/lucide" >/dev/null
fi

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
# Crow logs into the shot home's ~/Library/Logs; a search shot would list that log (it
# names the demo folders), so a demo home starts from none, and every shot logs only warnings.
if [ -n "${CROW_SHOT_HOME:-}" ]; then
  rm -f "$shot_home/Library/Logs/Crow/crow.log"
fi
RUST_LOG=${RUST_LOG:-warn} HOME="$shot_home" CROW_CONFIG_DIR="$work/config" CROW_STATE_DIR="$work/state" PATH="$work/bin:/usr/bin:/bin" \
  "$app" ${CROW_SHOT_ARGS:-} "$(cd "$start_dir" && pwd -P)" >"$work/log" 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; rm -rf "$work"' EXIT

# The swift helpers run as compiled binaries (cached in target/shots/bin): the interpreter's
# start-up cost about a second each, per shot.
tool() {
  local bin="$root/target/shots/bin/$1"
  if [ ! -x "$bin" ] || [ "$root/scripts/$1.swift" -nt "$bin" ]; then
    mkdir -p "$root/target/shots/bin"
    swiftc -O -o "$bin" "$root/scripts/$1.swift" 2>/dev/null || return 1
  fi
  printf '%s' "$bin"
}
window_id=$(tool window-id) || window_id=""
lights_active=$(tool lights-active) || lights_active=""
run_tool() { # <binary or empty> <script name> args...: the compiled helper, else the interpreter
  local bin=$1 name=$2
  shift 2
  if [ -n "$bin" ]; then "$bin" "$@"; else swift "$root/scripts/$name.swift" "$@"; fi
}

window=""
for _ in $(seq 1 150); do
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "shot.sh: Crow exited; log:" >&2
    cat "$work/log" >&2
    exit 1
  fi
  window=$(run_tool "$window_id" window-id "$pid" 2>/dev/null || true)
  [ -n "$window" ] && break
  sleep 0.2
done
if [ -z "$window" ]; then
  echo "shot.sh: no window for pid $pid after 30 s" >&2
  exit 1
fi
# A window caught while it is still opening is smaller; give it a moment to reach its size.
for _ in $(seq 1 25); do
  [ "${window##* }" = "1280x800" ] && break
  sleep 0.2
  window=$(run_tool "$window_id" window-id "$pid" 2>/dev/null || echo "$window")
done
id=${window%% *}
size=${window##* }
if [ "$size" != "1280x800" ]; then
  echo "shot.sh: window is $size, expected 1280x800 (is another Crow running?)" >&2
fi

echo "Crow pid $pid, window $id ($size), theme $theme, keymap $keymap"

# Bring Crow forward once before the keys, then check before every input that it still is.
# Tabbing out to another app cancels the run: the keys must never land in something else.
bring_to_front() {
  osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $pid) to true" >/dev/null 2>&1 || true
}
frontmost_pid() {
  osascript -e 'tell application "System Events" to get unix id of first process whose frontmost is true' 2>/dev/null || true
}

press() {
  case $1 in
    wait:*) ;;
    *)
      if [ "$(frontmost_pid)" != "$pid" ]; then
        echo "shot.sh: Crow is no longer the frontmost app; stopping before '$1'" >&2
        exit 1
      fi
      ;;
  esac
  case $1 in
    text:*) osascript -e "tell application \"System Events\" to keystroke \"${1#text:}\"" ;;
    key:* | shift:* | hold:*)
      local name=${1#*:}
      name=${name%%:*}
      case $name in
        return) code=36 ;;
        escape) code=53 ;;
        tab) code=48 ;;
        space) code=49 ;;
        delete) code=51 ;;
        up) code=126 ;;
        down) code=125 ;;
        left) code=123 ;;
        right) code=124 ;;
        f2) code=120 ;;
        home) code=115 ;;
        end) code=119 ;;
        pageup) code=116 ;;
        pagedown) code=121 ;;
        *)
          echo "shot.sh: unknown key ${1#key:}" >&2
          return 1
          ;;
      esac
      if [ "${1%%:*}" = hold ]; then
        osascript -e "tell application \"System Events\" to repeat ${1##*:} times" \
          -e "key code $code" -e "delay 0.02" -e "end repeat"
      elif [ "${1%%:*}" = shift ]; then
        osascript -e "tell application \"System Events\" to key code $code using shift down"
      else
        osascript -e "tell application \"System Events\" to key code $code"
      fi
      ;;
    cmd:*) osascript -e "tell application \"System Events\" to keystroke \"${1#cmd:}\" using command down" ;;
    ctrlcmd:*) osascript -e "tell application \"System Events\" to keystroke \"${1#ctrlcmd:}\" using {control down, command down}" ;;
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

recorder=""
if [ -n "${CROW_SHOT_RECORD:-}" ]; then
  rec_bin="$root/target/shots/record-window"
  if [ ! -x "$rec_bin" ] || [ "$root/scripts/record-window.swift" -nt "$rec_bin" ]; then
    swiftc -O -o "$rec_bin" "$root/scripts/record-window.swift"
  fi
  movie="$root/target/shots/$state-$theme.mov"
  rm -f "$movie"
  osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $pid) to true" >/dev/null 2>&1 || true
  sleep 0.8
  "$rec_bin" "$id" "$movie" >"$work/rec" 2>&1 &
  recorder=$!
  for _ in $(seq 1 40); do
    grep -q recording "$work/rec" 2>/dev/null && break
    kill -0 "$recorder" 2>/dev/null || { cat "$work/rec" >&2; exit 1; }
    sleep 0.1
  done
fi

# Stills need less settling than a recording, whose clips are paced by these gaps.
if [ -n "$recorder" ]; then lead=0.8 gap=0.35 tail=0.8; else lead=0.3 gap=0.12 tail=0.4; fi
if [ -n "$keys" ]; then
  bring_to_front
  sleep "$lead"
  IFS='|' read -ra steps <<<"$keys"
  for step in "${steps[@]}"; do
    press "$step"
    sleep "$gap"
  done
  sleep "$tail"
elif [ -n "$recorder" ]; then
  sleep "${CROW_SHOT_RECORD_SECONDS:-3}"
else
  echo "Set up the '$state' state in the window, then press Return to capture."
  # A piped or closed stdin is an immediate Return (scripted runs).
  read -r _ || true
fi

if [ -n "${CROW_SHOT_OTHER_WINDOW:-}" ]; then
  other=$(run_tool "$window_id" window-id "$pid" "$id" 2>/dev/null || true)
  if [ -z "$other" ]; then
    echo "shot.sh: the keys opened no other window" >&2
    exit 1
  fi
  id=${other%% *}
fi

if [ -n "$recorder" ]; then
  kill -INT "$recorder"
  wait "$recorder" || true
  [ -s "$movie" ] || { echo "shot.sh: no recording" >&2; cat "$work/rec" >&2; exit 1; }
  echo "wrote $movie"
  exit 0
fi

out="$root/target/shots/$state-$theme.png"
# Grey traffic lights mean another app took focus; bring Crow forward and retake (a few times).
captured=""
for attempt in 1 2 3 4; do
  osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $pid) to true" >/dev/null 2>&1 || true
  sleep 0.3
  if ! screencapture -x -o -l "$id" "$out" 2>/dev/null; then
    echo "shot.sh: screencapture failed (attempt $attempt); retrying" >&2
    continue
  fi
  if run_tool "$lights_active" lights-active "$out" 2>/dev/null; then captured=yes; break; fi
  echo "shot.sh: the window was not active (attempt $attempt); retrying" >&2
done
if [ -z "$captured" ]; then
  echo "shot.sh: no good capture of $state after 4 attempts (is the Mac asleep, locked, or another app in front? Screen Recording permission?)" >&2
  exit 1
fi
echo "wrote $out"
