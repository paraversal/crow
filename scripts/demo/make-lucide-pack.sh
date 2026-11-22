#!/usr/bin/env bash
# Build the "Lucide" demo icon pack: file and folder icons from the Lucide set (ISC
# licence), tinted with the theme's palette hues. Used by the persona shots.
#
#   scripts/demo/make-lucide-pack.sh [out-dir]     # default target/demo/lucide-pack
#   LUCIDE_ONLY=projects scripts/demo/make-lucide-pack.sh <out-dir>
#       just the project folders' icons, so everything else keeps the base icons
#
# Lucide is fetched once, pinned, from the npm registry into target/demo/lucide
# (not committed). Install the result by copying it to
# ~/.config/crow/icon-packs/lucide and choosing it in Settings > Appearance.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
out=${1:-$root/target/demo/lucide-pack}
version=1.77.0
sha=04322da4737e0f505646ac2dbbc273c8512b95ec737a462248621a2a32c65da1
cache=$root/target/demo/lucide
icons=$cache/package/icons

if [ ! -d "$icons" ]; then
  mkdir -p "$cache"
  curl -fsSL -A "crow-demo-builder/0.1" -o "$cache/lucide.tgz" \
    "https://registry.npmjs.org/lucide-static/-/lucide-static-$version.tgz"
  echo "$sha  $cache/lucide.tgz" | shasum -a 256 -c - >/dev/null
  tar xzf "$cache/lucide.tgz" -C "$cache" package/icons package/LICENSE
fi

rm -rf "$out"
mkdir -p "$out/icons"
cp "$cache/package/LICENSE" "$out/LICENSE-lucide.txt"
cp "$(dirname "$0")/lucide-pack-readme.md" "$out/README.md"
{
  echo '# Lucide icons (https://lucide.dev, ISC licence), tinted with the theme palette.'
  echo 'api = 1'
  echo 'id = "lucide"'
  echo 'name = "Lucide"'
} >"$out/pack.toml"

section=""
used=""
seen=""
only=${LUCIDE_ONLY:-}
force=""
# add <section> <key> <lucide icon> [color]; a key already added is skipped, and with
# LUCIDE_ONLY set only `project` entries are written.
add() {
  local sec=$1 key=$2 icon=$3 color=${4:-}
  [ -z "$only" ] || [ -n "$force" ] || return 0
  case "$seen" in *"|$sec/$key|"*) return 0 ;; esac
  seen="$seen|$sec/$key|"
  [ -f "$icons/$icon.svg" ] || { echo "make-lucide-pack: no Lucide icon '$icon'" >&2; exit 1; }
  if [ "$sec" != "$section" ]; then printf '\n[%s]\n' "$sec" >>"$out/pack.toml"; section=$sec; fi
  case " $used " in *" $icon "*) ;; *)
    # Plain SVG: no comment, no class, so the pack loader sees only drawing elements.
    sed -e '/<!--/d' -e 's/ class="[^"]*"//' "$icons/$icon.svg" >"$out/icons/$icon.svg"
    used="$used $icon" ;;
  esac
  if [ -n "$color" ]; then
    printf '"%s" = { icon = "icons/%s.svg", color = "%s" }\n' "$key" "$icon" "$color" >>"$out/pack.toml"
  else
    printf '"%s" = { icon = "icons/%s.svg" }\n' "$key" "$icon" >>"$out/pack.toml"
  fi
}

project() { force=1 add folder_name "$@"; }

# --- project folders: the demo home's repositories and the personas' ------------------
project tidepool waves base0C
project field-guide bird base0E
project raven-bot bot base0D
project garden-planner sprout base0B
project dotfiles terminal base0B
project homelab server base09
project recipe-api chef-hat base08
project prod-eu server base08
project staging server-cog base0A
project bastion shield base0E
project raven-cam camera base0E
project synth-rs keyboard-music base0F
project wasm-life dna base0B
project advent-of-code tree-pine base09
project 2026 calendar-days base09
project "2026-03 Lofoten" mountain-snow base0C
project "2026-05 Dolomites" mountain base0F
project "2026-10 Fog Valley" cloud-fog base0D
project "2026-07 Faroe Islands" sailboat base0B
project "Anna and Tom" heart base08
project "Atelier Noor" palette base09
project "Nordic Outdoors" compass base0C
project "00 Notes" notebook-pen base0A
project Drafts file-pen base0D
project Fiction moon base0E
project Craft hammer base0B
project Poetry flower-2 base0F
project "Sample Library" audio-waveform base09
project "Night Drive" car base0D
project "Glass Rain" cloud-rain base0C
project "Violet Hour" sunset base0E
project Kicks circle-dot base08
project Hats audio-lines base09
project Pads cloud base0C
project Foley door-open base0B
project Games gamepad-2 base0E

# --- folders by name ------------------------------------------------------------
for n in src lib crates app; do add folder_name "$n" folder-code base0D; done
add folder_name tests folder-check base0B
add folder_name test folder-check base0B
add folder_name docs folder-open base0C
add folder_name .git folder-git-2 base09
add folder_name .github folder-git-2 base09
add folder_name node_modules folder-archive fg-muted
add folder_name target folder-archive fg-muted
add folder_name dist folder-archive fg-muted
add folder_name build folder-archive fg-muted
add folder_name .config folder-cog base0E
add folder_name config folder-cog base0E
add folder_name terraform folder-cog base0E
add folder_name Desktop monitor base0D
add folder_name Documents file-text base0A
add folder_name Downloads folder-down base0B
add folder_name Pictures image base0E
add folder_name Music music base08
add folder_name Movies film base0C
add folder_name Projects folder-kanban base09
add folder_name Servers server base0D
add folder_name Volumes hard-drive base0C
add folder_name Clients users base0D
add folder_name Contracts file-signature base0A
add folder_name Invoices receipt base0B
add folder_name Delivered package-check base0B
add folder_name Moodboards layout-grid base0E
add folder_name Catalogs library base0A
add folder_name Presets sliders-horizontal base0E
add folder_name Gear camera base09
add folder_name RAW aperture base08
add folder_name Selects star base0A
add folder_name Edits wand-sparkles base0E
add folder_name Exports share base0B
add folder_name Portfolio gallery-horizontal base0E
add folder_name Prints printer base09
add folder_name Manuscripts book-open base0A
add folder_name Chapters bookmark base09
add folder_name Research microscope base0C
add folder_name Journal notebook-pen base0B
add folder_name Submissions send base0D
add folder_name Essays feather base0E
add folder_name Reading library-big base0C
add folder_name Newsletter mail base0D
add folder_name Samples audio-waveform base0E
add folder_name Stems layers base0D
add folder_name Bounces disc-3 base0C
add folder_name Mixdowns sliders-vertical base08
add folder_name Releases disc-album base0A
add folder_name Gigs mic-vocal base08
add folder_name MIDI piano base0B
add folder_name Plugins plug base0F
add folder_name Drums drum base08
add folder_name Synths keyboard-music base0E
add folder_name Vocals mic base09
add folder_name Notes notebook-pen base0B
add folder_name dotfiles terminal base0B

# --- volumes (mock external drives, shown as folders) ---------------------------------
for v in "Build SSD" "Archive SSD" "Backup RAID" "Backups" "Session Drive" "Sample Library SSD" \
  "Studio Backup" "Backup stick" "Typewriter archive" "EOS R5 Card" "Games"; do
  add folder_name "$v" hard-drive base0C
done

# --- files by extension -----------------------------------------------------------------
for e in rs go c cpp h swift; do add extension $e file-code base09; done
for e in ts tsx js jsx mjs; do add extension $e file-code base0D; done
for e in py rb lua; do add extension $e file-code base0A; done
add extension html file-code base08
add extension css file-code base0C
add extension json file-json base0A
add extension toml file-cog base0C
add extension yaml file-cog base0C
add extension yml file-cog base0C
add extension tf file-cog base0E
add extension sh file-terminal base0B
add extension zsh file-terminal base0B
add extension lock lock fg-muted
add extension env key-round base0A
add extension log scroll-text fg-muted
add extension sql database base0D
add extension db database base0D
add extension sqlite database base0D
add extension md file-text base0D
add extension txt file-text fg
add extension rtf file-text fg
add extension docx file-text base0D
add extension pages file-text base09
add extension pdf file-text base08
add extension epub book-open base0C
add extension csv table base0B
add extension xlsx table base0B
for e in jpg jpeg png tif tiff webp heic psd; do add extension $e image base0E; done
for e in cr3 cr2 arw nef dng raf; do add extension $e aperture base08; done
add extension xmp sliders-horizontal base0E
add extension lrcat library base0A
for e in mp4 mov mkv; do add extension $e film base0C; done
for e in wav aif aiff flac; do add extension $e audio-lines base0E; done
for e in mp3 m4a; do add extension $e music base08; done
for e in als logicx flp; do add extension $e audio-waveform base0A; done
add extension mid piano base0B
add extension midi piano base0B
add extension fxp sliders-vertical base0F
add extension zip archive base09
add extension tar archive base09
add extension gz archive base09
add extension dmg disc fg-muted
add extension ttf type base0D
add extension otf type base0D
add extension stl box base0B

# --- generic ----------------------------------------------------------------------------------
[ -z "$only" ] || { echo "built $out ($(ls "$out"/icons/*.svg | wc -l | tr -d ' ') icons, projects only)"; exit 0; }
printf '\n[generic]\n' >>"$out/pack.toml"
add_generic() { # name icon color  (add() writes a [section]; generic keys are bare)
  [ -f "$icons/$2.svg" ] || { echo "make-lucide-pack: no Lucide icon '$2'" >&2; exit 1; }
  sed -e '/<!--/d' -e 's/ class="[^"]*"//' "$icons/$2.svg" >"$out/icons/$2.svg"
  printf '%s = { icon = "icons/%s.svg", color = "%s" }\n' "$1" "$2" "$3" >>"$out/pack.toml"
}
add_generic folder folder base0D
add_generic file file fg-muted

echo "built $out ($(ls "$out"/icons/*.svg | wc -l | tr -d ' ') icons)"
