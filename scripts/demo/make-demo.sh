#!/usr/bin/env bash
# Build target/demo/home: a small, believable home folder for screenshots and clips.
# Nothing in it comes from this Mac: notes and Git history are generated, pictures are
# public-domain photos from pxhere (photos.tsv),
# names are made up, and every file gets a fixed date so shots can be retaken.
#
#   scripts/demo/make-demo.sh [out-dir]     # default target/demo/home, rebuilt from scratch
#
# Shots run Crow with HOME set to this folder, so the path bar reads ~/...
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
home=${1:-$root/target/demo/home}
rm -rf "$home"
mkdir -p "$home"
cd "$home"

stamp=202611141030 # touch -t: every file, unless a line below says otherwise

note() { mkdir -p "$(dirname "$1")"; cat >"$1"; }

# --- Documents -------------------------------------------------------------
note Documents/Notes/Reading\ list.md <<'EOF'
# Reading list

- The Peregrine — J. A. Baker
- A Pattern Language — Christopher Alexander
- The Design of Everyday Things — Don Norman
- Mind of the Raven — Bernd Heinrich
EOF
note Documents/Notes/Ideas.md <<'EOF'
# Ideas

- Shelf for the hallway, 90 cm, oak
- Weekend in Porto in March?
- Learn the Vim motions properly
- Bird feeder camera
EOF
note Documents/Notes/Groceries.md <<'EOF'
- oats
- lemons
- coffee beans
- olive oil
EOF
note Documents/Recipes/Shakshuka.md <<'EOF'
# Shakshuka

1. Soften an onion and a red pepper in olive oil.
2. Add garlic, cumin and paprika, then a tin of tomatoes.
3. Simmer ten minutes, make wells, crack in four eggs.
4. Cover until the whites set. Parsley, feta, bread.
EOF
note Documents/Recipes/Lemon\ cake.md <<'EOF'
# Lemon cake

200 g butter, 200 g sugar, 4 eggs, 200 g flour, 2 lemons.
EOF
mkdir -p "Documents/Taxes 2025"
for doc in "Income statement" "Receipts" "Return summary"; do
  printf '%s\n\nTax year 2025\n' "$doc" >"/tmp/crow-demo-$$.txt"
  cupsfilter -m application/pdf "/tmp/crow-demo-$$.txt" >"Documents/Taxes 2025/$doc.pdf" 2>/dev/null
done
rm -f "/tmp/crow-demo-$$.txt"
printf 'Dear Ana,\n\nThank you for the postcard from Lisbon.\n' >Documents/Letter\ to\ Ana.txt
textutil -convert rtf Documents/Letter\ to\ Ana.txt -output Documents/Letter\ to\ Ana.rtf >/dev/null
rm Documents/Letter\ to\ Ana.txt
printf 'Flat 3B lease renewal\n' >"/tmp/crow-demo-$$.txt"
cupsfilter -m application/pdf "/tmp/crow-demo-$$.txt" >Documents/Lease.pdf 2>/dev/null
rm -f "/tmp/crow-demo-$$.txt"

# --- Pictures --------------------------------------------------------------
# Real mountain photos, all public domain (CC0) from pxhere (photos.tsv lists the source).
# They are downloaded once into target/demo/photos, scaled to 1920 px, and copied from there.
photos=$root/target/demo/photos
mkdir -p "$photos"
grep -v '^#' "$root/scripts/demo/photos.tsv" | while IFS=$'\t' read -r rel _ lic _ url; do
  [ "$lic" = CC0 ] || { echo "make-demo: $rel is not CC0" >&2; exit 1; }
  cached="$photos/$(basename "$rel")"
  if [ ! -s "$cached" ]; then
    curl -fsSL -A "crow-demo-builder/0.1" -o "$cached.part" "$url"
    sips -Z 1920 "$cached.part" --out "$cached" >/dev/null && rm "$cached.part"
    sleep 1 # be gentle with the host
  fi
  mkdir -p "Pictures/$(dirname "$rel")"
  cp "$cached" "Pictures/$rel"
done
mkdir -p Pictures/Screenshots
cp "Pictures/Wallpapers/Blue sky.jpg" "Pictures/Screenshots/Screenshot 2026-11-12 at 09.14.02.jpg"
cp "Pictures/Trips/Lake shore.jpg" "Pictures/Screenshots/Screenshot 2026-11-13 at 17.40.51.jpg"

# --- Downloads -------------------------------------------------------------
mkdir -p Downloads
printf 'Invoice 0412\nAmount due: 120.00\n' >"/tmp/crow-demo-$$.txt"
cupsfilter -m application/pdf "/tmp/crow-demo-$$.txt" >Downloads/invoice-0412.pdf 2>/dev/null
rm -f "/tmp/crow-demo-$$.txt"
(cd Documents && zip -qr ../Downloads/recipes.zip Recipes)
printf 'id,species,count\n1,raven,4\n2,jackdaw,11\n3,rook,7\n' >Downloads/birds.csv
printf '#!/bin/sh\necho hello\n' >Downloads/install
chmod +x Downloads/install

# --- Desktop, Music, Movies -----------------------------------------------
mkdir -p Desktop Music Movies
cp "Pictures/Wallpapers/Turquoise lake.jpg" "Desktop/Wallpaper.jpg"
printf 'Call the landlord\n' >Desktop/todo.txt
ln -s ../Documents/Notes Desktop/Notes
cp "Pictures/Trips/Meadow.jpg" "Desktop/Screenshot 2026-11-14 at 10.02.11.jpg"
printf 'Dentist Tue 9:30\nReturn library books\n' >Desktop/reminders.txt

# --- More of everything: a home that has been lived in ---------------------
pdf() { # pdf <out> <text>
  printf '%s\n' "$2" >"/tmp/crow-demo-$$.txt"
  cupsfilter -m application/pdf "/tmp/crow-demo-$$.txt" >"$1" 2>/dev/null
  rm -f "/tmp/crow-demo-$$.txt"
}
for n in "Book club" "Garden plan" "Packing list" "Gift ideas" "Podcast notes" "Running log"; do
  printf '# %s\n\n- \n' "$n" >"Documents/Notes/$n.md"
done
for n in "Banana bread" "Ramen" "Pesto" "Dal"; do
  printf '# %s\n' "$n" >"Documents/Recipes/$n.md"
done
mkdir -p Documents/Travel Documents/Invoices Documents/Work
printf '# Lisbon, 12–16 March\n\n- Thu: Alfama, tram 28\n- Fri: Sintra\n- Sat: LX Factory\n' >Documents/Travel/Lisbon.md
pdf Documents/Travel/Boarding\ pass\ LIS.pdf "Boarding pass"
pdf Documents/Travel/Hotel\ booking.pdf "Hotel booking"
for m in 01 02 03 04 05 06 07 08 09 10; do pdf "Documents/Invoices/2026-$m.pdf" "Invoice 2026-$m"; done
pdf Documents/Work/Quarterly\ review.pdf "Quarterly review"
printf 'month,rent,food,travel\nJan,900,310,0\nFeb,900,295,120\nMar,900,330,640\n' >Documents/Budget\ 2026.csv
pdf Documents/CV.pdf "Curriculum vitae"
printf 'Warranty: dishwasher, until 2028\n' >Documents/Warranty.txt

pdf Downloads/Train\ tickets.pdf "Train tickets"
pdf Downloads/manual-espresso-machine.pdf "Espresso machine manual"
cp "Pictures/Golden forest.jpg" Downloads/IMG_4021.jpg
cp "Pictures/Trips/Larches.jpg" Downloads/IMG_4022.jpg
printf '{ "theme": "finder-dark" }\n' >Downloads/settings-backup.json
printf 'BEGIN:VCALENDAR\nEND:VCALENDAR\n' >Downloads/invite.ics
(cd Documents && zip -qr ../Downloads/notes-backup.zip Notes)
mkdir -p "Downloads/old"

printf '# Playlists\n\nSunday morning\nFocus\n' >Music/Playlists.md
printf 'placeholder\n' >Movies/.keep

# --- Projects: two Git repos with uncommitted changes ----------------------
git_demo() { GIT_AUTHOR_NAME=Demo GIT_AUTHOR_EMAIL=demo@example.com GIT_COMMITTER_NAME=Demo \
  GIT_COMMITTER_EMAIL=demo@example.com GIT_AUTHOR_DATE="2026-11-10T09:00:00" \
  GIT_COMMITTER_DATE="2026-11-10T09:00:00" git -c init.defaultBranch=main -c commit.gpgsign=false "$@"; }

mkdir -p Projects/tidepool/src
cd Projects/tidepool
note README.md <<'EOF'
# tidepool

Logs tide heights from a cheap pressure sensor and draws a week at a glance.

```sh
cargo run --release -- --port /dev/tty.usbserial
```

| Command | What it does |
| --- | --- |
| `record` | Reads the sensor every minute |
| `plot`   | Draws the last seven days |
EOF
note Cargo.toml <<'EOF'
[package]
name = "tidepool"
version = "0.3.0"
edition = "2021"
EOF
note src/main.rs <<'EOF'
mod sensor;

fn main() {
    let reading = sensor::read().expect("sensor");
    println!("{:.2} m", reading.height());
}
EOF
note src/sensor.rs <<'EOF'
pub struct Reading(f64);

impl Reading {
    pub fn height(&self) -> f64 {
        self.0
    }
}

pub fn read() -> Option<Reading> {
    Some(Reading(1.42))
}
EOF
printf '/target\n' >.gitignore
git_demo init -q && git_demo add -A && git_demo commit -qm "First reading"
cat >>src/sensor.rs <<'EOF'

pub fn calibrate(offset: f64) -> Reading {
    Reading(1.42 + offset)
}
EOF
note src/plot.rs <<'EOF'
pub fn week() {}
EOF
mkdir -p target/release && printf 'build output\n' >target/release/tidepool
cd "$home"

mkdir -p Projects/field-guide
cd Projects/field-guide
note index.html <<'EOF'
<!doctype html>
<title>Field guide</title>
<h1>Corvids of the city park</h1>
EOF
note style.css <<'EOF'
body { font: 16px/1.5 system-ui; margin: 2rem; }
EOF
note birds.json <<'EOF'
[{ "name": "Raven" }, { "name": "Jackdaw" }, { "name": "Rook" }]
EOF
git_demo init -q && git_demo add -A && git_demo commit -qm "Start the guide"
printf 'h1 { color: #222; }\n' >>style.css
cd "$home"

mkdir -p Projects/raven-bot/raven_bot Projects/raven-bot/tests
cd Projects/raven-bot
printf '# raven-bot\n\nPosts the day'"'"'s bird count to the group chat.\n' >README.md
printf '[project]\nname = "raven-bot"\nversion = "0.1.0"\n' >pyproject.toml
printf 'def count(rows):\n    return sum(r["count"] for r in rows)\n' >raven_bot/__init__.py
printf 'from raven_bot import count\n\ndef test_count():\n    assert count([{"count": 2}]) == 2\n' >tests/test_count.py
git_demo init -q && git_demo add -A && git_demo commit -qm "Count birds"
mkdir -p .venv/lib && printf 'venv\n' >.venv/pyvenv.cfg
cd "$home"

mkdir -p Projects/garden-planner/src Projects/garden-planner/node_modules/left-pad
cd Projects/garden-planner
printf '{ "name": "garden-planner", "version": "1.2.0" }\n' >package.json
printf 'export const beds = ["herbs", "tomatoes", "beans"];\n' >src/beds.ts
printf 'import { beds } from "./beds";\nconsole.log(beds);\n' >src/main.ts
printf 'node_modules\n' >.gitignore
printf 'module.exports = s => s;\n' >node_modules/left-pad/index.js
git_demo init -q && git_demo add -A && git_demo commit -qm "Beds"
printf 'export const season = "spring";\n' >src/season.ts
git_demo add src/season.ts
cd "$home"

mkdir -p Projects/dotfiles/zsh Projects/dotfiles/git
cd Projects/dotfiles
printf 'alias ll="ls -l"\n' >zsh/aliases.zsh
printf '[user]\n\tname = Demo\n' >git/config
git_demo init -q && git_demo add -A && git_demo commit -qm "Dotfiles"
cd "$home"

# --- A fuller life: more repos, paperwork, music, camera roll --------------
mkdir -p Projects/homelab/compose Projects/homelab/ansible
cd Projects/homelab
printf '# homelab\n\nCompose files and playbooks for the Raspberry Pi in the hallway.\n' >README.md
printf 'services:\n  pihole:\n    image: pihole/pihole\n    restart: unless-stopped\n' >compose/dns.yml
printf 'services:\n  photos:\n    image: immich\n' >compose/photos.yml
printf -- '- hosts: pi\n  tasks:\n    - name: Update packages\n      apt: upgrade=yes\n' >ansible/update.yml
git_demo init -q && git_demo add -A && git_demo commit -qm "Pi-hole and photos"
printf 'services:\n  vault:\n    image: vaultwarden/server\n' >compose/vault.yml
printf '# homelab\n\nCompose files and playbooks for the Raspberry Pi in the hallway.\n\n## Backups\nNightly to the NAS.\n' >README.md
cd "$home"

mkdir -p Projects/recipe-api/cmd Projects/recipe-api/internal
cd Projects/recipe-api
printf 'module example.com/recipe-api\n\ngo 1.23\n' >go.mod
printf 'package main\n\nfunc main() {}\n' >cmd/main.go
printf 'package recipes\n\ntype Recipe struct{ Name string }\n' >internal/recipes.go
printf '# recipe-api\n\nServes the recipes in ~/Documents/Recipes as JSON.\n' >README.md
git_demo init -q && git_demo add -A && git_demo commit -qm "Serve recipes"
cd "$home"

mkdir -p Projects/scratch Projects/archive/old-site
printf 'print("scratch")\n' >Projects/scratch/test.py
printf 'todo: try the new tide model\n' >Projects/scratch/notes.txt
printf '<h1>Old site</h1>\n' >Projects/archive/old-site/index.html

mkdir -p Documents/Home Documents/Health Documents/Learning Documents/Writing Documents/Archive/2024
pdf "Documents/Home/Contents insurance.pdf" "Contents insurance policy"
pdf "Documents/Home/Energy bill 2026-10.pdf" "Energy bill October"
pdf "Documents/Home/Floor plan.pdf" "Floor plan, flat 3B"
pdf "Documents/Health/Dentist invoice.pdf" "Dentist invoice"
pdf "Documents/Health/Vaccination record.pdf" "Vaccination record"
pdf "Documents/Archive/2024/Tax return 2024.pdf" "Tax return 2024"
pdf "Documents/Archive/2024/Old lease.pdf" "Lease 2022 to 2024"
printf '# Rust notes\n\n- ownership, borrowing, lifetimes\n- `?` for errors\n' >"Documents/Learning/Rust notes.md"
printf '# Vim cheatsheet\n\n- `ciw` change inner word\n- `gg` / `G` top, bottom\n' >"Documents/Learning/Vim cheatsheet.md"
printf '# Spanish vocab\n\nel cuervo, la urraca, el halcón\n' >"Documents/Learning/Spanish vocab.md"
printf '# Logging tides on a budget\n\nDraft. A pressure sensor, a Pi and a week of patience.\n' >"Documents/Writing/Blog draft.md"
printf '# Newsletter, issue 4\n\nBirds of the month: the rook.\n' >"Documents/Writing/Newsletter 04.md"

mkdir -p "Pictures/Camera Roll"
i=4100
for photo in "Golden forest.jpg" "Trips/Larches.jpg" "Trips/Meadow.jpg" "Trips/Lake shore.jpg" "Wallpapers/Blue sky.jpg" "Wallpapers/Turquoise lake.jpg"; do
  cp "Pictures/$photo" "Pictures/Camera Roll/IMG_$i.jpg"
  i=$((i + 3))
done

mkdir -p "Music/Field recordings"
python3 - <<'PY'
import wave
for name, seconds in (("Raven at dawn", 2), ("Rain on the roof", 3), ("Harbour", 2)):
    with wave.open(f"Music/Field recordings/{name}.wav", "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(8000)
        w.writeframes(b"\x00\x00" * 8000 * seconds)
PY

# --- Tags (Finder's xattr) -------------------------------------------------
tag() { # tag <file> <name>\n<color> ...
  local f=$1; shift
  local items=""
  for t in "$@"; do items="$items<string>$t</string>"; done
  printf '<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><array>%s</array></plist>' "$items" |
    plutil -convert binary1 -o - - | xxd -p | tr -d '\n' |
    xargs -I{} xattr -wx com.apple.metadata:_kMDItemUserTags {} "$f"
}
tag "Documents/Taxes 2025" $'Important\n2'
tag Documents/Notes/Ideas.md $'Home\n4'
tag "Pictures/Trips/Jagged peaks.jpg" $'Travel\n6'
tag "Pictures/Trips/Lake at dusk.jpg" $'Travel\n6'

# --- Fixed dates -----------------------------------------------------------
# Spread over a few days so the Date column reads like a real folder; Git's own files keep theirs.
i=0
find . -path '*/.git' -prune -o -print | sort | while read -r p; do
  day=$((10 + i % 7))
  hour=$((9 + i % 9))
  touch -h -t "$(printf '202611%02d%02d%02d' "$day" "$hour" $((i * 7 % 60)))" "$p"
  i=$((i + 1))
done
# A newer export of a note in Downloads, for the copy-conflict shot (pasted into
# Documents/Notes). Written after the fixed dates so it shifts no other file's date.
cp Documents/Notes/Ideas.md Downloads/Ideas.md
printf -- '- Ask about the allotment waiting list\n' >>Downloads/Ideas.md
touch -t 202611180915 Downloads/Ideas.md
touch -t "$stamp" .
echo "demo home: $home"
