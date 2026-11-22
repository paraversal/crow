#!/usr/bin/env bash
# Build four home folders that look lived in, for the website's "Make it yours" shots:
# the zealous developer, the photographer, the writer and the producer. Each has a deep
# folder tree (hundreds of entries, real Git repos for the developer) so Crow is shown
# pointed at an actual file tree, and mock external drives as plain folders under
# Volumes/ (a sidebar group named Locations lists them; the real Locations section is off).
#
#   scripts/demo/make-personas.sh [parent-dir] [persona...]
#
# Default parent /Users/Shared (the path bar then reads Macintosh HD > Users > Shared >
# Demo-developer). Personas: developer photographer writer producer (default: all).
# Nothing comes from this Mac: files are empty or tiny, sizes are sparse, dates are fixed,
# names are made up, and the pictures are the public-domain photos scripts/demo/photos.tsv
# lists (run scripts/demo/make-demo.sh once so they are cached).
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
parent=${1:-/Users/Shared}
shift || true
want=${*:-developer photographer writer producer}

photos=$root/target/demo/photos
ph=()
while IFS= read -r f; do ph+=("$f"); done < <(find "$photos" -name '*.jpg' 2>/dev/null | sort)
if [ "${#ph[@]}" -lt 8 ]; then
  echo "make-personas: run scripts/demo/make-demo.sh first (it downloads the photos)" >&2
  exit 1
fi

base=202611141030
base_epoch=$(/bin/date -j -f %Y%m%d%H%M "$base" +%s)

# A deterministic number per path, so every run makes the same dates and sizes.
num() { printf '%s' "$1" | cksum | cut -d' ' -f1; }
when() { # <path> -> touch -t stamp, within the 14 months before $base
  local n
  n=$(num "$1")
  /bin/date -r $((base_epoch - (n % 420) * 86400 - (n % 11) * 3600 - (n % 47) * 60)) +%Y%m%d%H%M
}

sentences=(
  "The ferry was late again, and nobody on the quay seemed to mind."
  "She kept the lamp trimmed out of habit, not hope."
  "By March the salt had got into everything: the books, the bread, the letters."
  "He wrote it down twice so that one of them would be true."
  "Nothing in the logbook explained the missing week."
  "Rain came in sideways, and the harbour smelled of tar and oranges."
)

# mk <path>...: make each file (and its folders). Text-like files get a line of text,
# everything else a sparse file of a believable size.
mk() {
  local f n ext size
  for f in "$@"; do
    mkdir -p "$(dirname "$f")"
    n=$(num "$f")
    ext=${f##*.}
    case $ext in
      rs|ts|tsx|js|py|go|sh|toml|yaml|yml|json|tf|css|html|sql|lua|csv|conf|zsh|kdl)
        printf '// %s\n' "$(basename "$f")" >"$f" ;;
      md|txt)
        printf '# %s\n\n%s\n' "$(basename "${f%.*}")" "${sentences[$((n % ${#sentences[@]}))]}" >"$f" ;;
      cr3|cr2|dng|arw) truncate -s $(((n % 12 + 18) * 1048576)) "$f" ;;
      wav|aif|aiff|flac) truncate -s $(((n % 38 + 3) * 1048576)) "$f" ;;
      mov|mp4|mkv) truncate -s $(((n % 700 + 80) * 1048576)) "$f" ;;
      lrcat|als|logicx|dmg) truncate -s $(((n % 60 + 20) * 1048576)) "$f" ;;
      tif|tiff|psd) truncate -s $(((n % 90 + 30) * 1048576)) "$f" ;;
      *) truncate -s $(((n % 900 + 40) * 1024)) "$f" ;;
    esac
    touch -t "$(when "$f")" "$f"
  done
}

# img <dest> <photo index> [variant]: a real photo, scaled and cropped so a folder of
# them is not twelve copies of the same picture.
img() {
  local d=$1 i=$(($2 % ${#ph[@]})) v=${3:-0}
  mkdir -p "$(dirname "$d")"
  sips -Z 1400 "${ph[$i]}" --out "$d" >/dev/null
  case $((v % 4)) in
    1) sips -c 800 800 "$d" >/dev/null 2>&1 || true ;;
    2) sips -c 600 1200 "$d" >/dev/null 2>&1 || true ;;
    3) sips -c 700 1000 "$d" >/dev/null 2>&1 || true ;;
  esac
  touch -t "$(when "$d")" "$d"
}
thumb() { # <dest> <source image>: a smaller export
  mkdir -p "$(dirname "$1")"
  sips -Z 900 "$2" --out "$1" >/dev/null
  touch -t "$(when "$1")" "$1"
}

git_demo() {
  GIT_AUTHOR_NAME=Demo GIT_AUTHOR_EMAIL=demo@example.com GIT_COMMITTER_NAME=Demo \
    GIT_COMMITTER_EMAIL=demo@example.com GIT_AUTHOR_DATE="2026-11-10T09:00:00" \
    GIT_COMMITTER_DATE="2026-11-10T09:00:00" git -c init.defaultBranch=main -c commit.gpgsign=false "$@"
}
# repo <dir> <changed-file> <new-file>: commit what is there, then leave one file
# modified and one untracked so the Git badges show.
repo() {
  (
    cd "$1"
    printf 'target\nnode_modules\ndist\n__pycache__\n.env\n' >.gitignore
    git_demo init -q
    git_demo add -A
    git_demo commit -qm "Initial commit"
    printf '// edited\n' >>"$2"
    printf '// new\n' >"$3"
  )
}

# ======================================================================================
developer() {
  local h=$parent/Demo-developer
  rm -rf "$h"; mkdir -p "$h"; cd "$h"

  local a=Projects/work/acme-api
  mk $a/{Cargo.toml,Cargo.lock,README.md,Dockerfile,justfile}
  mk $a/src/{main,lib,config,error,state,telemetry}.rs
  mk $a/src/routes/{mod,users,orders,invoices,health,webhooks}.rs
  mk $a/src/db/{mod,pool,queries}.rs $a/src/db/migrations/{0001_users,0002_orders,0003_invoices,0004_indexes,0005_audit}.sql
  mk $a/src/auth/{mod,jwt,session,roles}.rs
  mk $a/tests/{users,orders,invoices,auth}_test.rs $a/tests/fixtures/{user,order,invoice}.json
  mk $a/benches/{routes,serialize}.rs
  mk $a/docs/{architecture,runbook,api}.md
  mk $a/.github/workflows/{ci,release,audit}.yml
  mk $a/target/release/{acme-api,build.log} $a/target/debug/{acme-api,deps.log}
  repo $a src/main.rs src/routes/search.rs

  local w=Projects/work/acme-web
  mk $w/{package.json,tsconfig.json,vite.config.ts,README.md,.eslintrc.json}
  mk $w/src/{main,App,router}.tsx
  mk $w/src/components/{Button,Modal,Table,Nav,Toast,Avatar,Form,Tabs}.tsx
  mk $w/src/pages/{Dashboard,Orders,Invoices,Settings,Login}.tsx
  mk $w/src/hooks/{useAuth,useFetch,useDebounce}.ts $w/src/lib/{api,format,storage}.ts
  mk $w/src/styles/{globals,tokens}.css $w/public/{favicon.svg,robots.txt}
  mk $w/tests/{Button,Table,Form}.test.tsx
  mk $w/node_modules/{react,react-dom,vite,typescript,zod,eslint,vitest}/package.json
  mk $w/dist/index.html $w/dist/assets/{index-8f3a2c,vendor-1b7d90}.js $w/dist/assets/index-8f3a2c.css
  repo $w src/App.tsx src/pages/Reports.tsx

  local i=Projects/work/infra
  mk $i/terraform/{main,variables,outputs,versions}.tf
  mk $i/terraform/modules/{vpc,ecs,rds,dns,cdn}/{main,variables,outputs}.tf
  mk $i/ansible/{inventory,site}.yml
  mk $i/ansible/roles/{nginx,postgres,node_exporter}/tasks/main.yml
  mk $i/ansible/roles/nginx/templates/{api,web}.conf.j2
  mk $i/scripts/{deploy,rollback,backup,rotate-keys}.sh $i/README.md

  local p=Projects/work/data-pipeline
  mk $p/{pyproject.toml,README.md,Makefile} $p/src/pipeline/{__init__,ingest,transform,load,schedule}.py
  mk $p/tests/{test_ingest,test_transform,test_load}.py $p/sql/{daily,weekly,anomalies}.sql
  mk $p/notebooks/{exploration,cohorts}.ipynb $p/__pycache__/ingest.cpython-312.pyc

  local t
  t=Projects/toys/tidepool
  mk $t/{Cargo.toml,README.md} $t/src/{main,tide,moon,render}.rs $t/tests/tide.rs $t/target/release/tidepool
  repo $t src/main.rs src/wind.rs
  t=Projects/toys/raven-cam
  mk $t/{pyproject.toml,README.md} $t/raven/{__init__,camera,detect,notify}.py $t/models/crow-v2.onnx $t/tests/test_detect.py
  repo $t raven/camera.py raven/storage.py
  t=Projects/toys/synth-rs
  mk $t/{Cargo.toml,README.md} $t/src/{main,osc,filter,envelope,midi,ui}.rs $t/presets/{pad,lead,bass}.toml $t/target/release/synth-rs
  repo $t src/osc.rs src/lfo.rs
  t=Projects/toys/wasm-life
  mk $t/{package.json,Cargo.toml,index.html} $t/src/{lib,universe}.rs $t/www/{index,bootstrap}.js $t/pkg/wasm_life_bg.wasm
  repo $t src/lib.rs src/glider.rs
  mk Projects/toys/advent-of-code/20{23,24,25}/day{01,02,03,04,05,06,07,08,09,10,11,12}.rs
  mk Projects/toys/advent-of-code/20{23,24,25}/inputs/day{01,02,03,04,05,06}.txt
  mk Projects/oss/ratatui-widgets/{Cargo.toml,README.md,LICENSE} Projects/oss/ratatui-widgets/src/{lib,gauge,table,sparkline}.rs
  mk Projects/oss/helix-notes/{README.md,notes.scm,languages.toml}

  local s
  for s in prod-eu staging bastion; do
    local r=Servers/work-vps/$s
    mk $r/etc/nginx/nginx.conf $r/etc/nginx/sites-enabled/{api,web}.conf
    mk $r/var/log/nginx/{access,error}.log $r/var/log/app.log
    mk $r/home/deploy/.ssh/config $r/home/deploy/deploy.sh
    mk $r/opt/acme/releases/2026-11-{02,09,14}/{acme-api,VERSION}
  done

  local d=Projects/dotfiles
  mk $d/{install.sh,README.md,Brewfile}
  mk $d/.config/nvim/init.lua $d/.config/nvim/lua/{plugins,keymaps,lsp,ui}.lua
  mk $d/.config/helix/{config,languages}.toml
  mk $d/.config/zsh/{.zshrc,aliases.zsh,prompt.zsh}
  mk $d/.config/git/{config,ignore} $d/.config/kitty/kitty.conf $d/.config/ghostty/config
  repo $d install.sh .config/zsh/functions.zsh

  mk Documents/Notes/{"Rust lifetimes","Postgres tuning","Terraform tips","Vim motions","On-call checklist"}.md
  mk Documents/Cheatsheets/{git,tmux,sql,regex}.md
  mk Documents/Talks/{"Rustfest 2026","Local-first apps"}/{slides,notes}.md
  mk Downloads/{ghostty-1.2,tailscale-1.80,orbstack}.dmg Downloads/{terraform_1.9,node-v22}.zip Downloads/postgres-internals.pdf
  mk Desktop/{todo.md,scratch.rs,standup.txt}

  mk "Volumes/Build SSD/cargo-cache/registry/index/"{crates.db,config.json} "Volumes/Build SSD/docker-images/"{postgres-16,redis-7,acme-api-0412}.tar
  mk "Volumes/Build SSD/builds/2026-11-"{02,09,14}"/"{acme-api,acme-web.zip}
  mk Volumes/Backups/db-dumps/{prod,staging}-2026-11-{01,08,13}.sql.gz Volumes/Backups/dotfiles-backup.tar.gz
  mk Volumes/Backups/keys/{id_ed25519.pub,known_hosts}
  mk Volumes/Games/{Steam,Roms}/placeholder.txt
}

# ======================================================================================
photographer() {
  local h=$parent/Demo-photographer
  rm -rf "$h"; mkdir -p "$h"; cd "$h"

  local shoots=("2026-01 Iceland Winter" "2026-02 Tokyo Streets" "2026-03 Lofoten" "2026-04 Wedding Anna and Tom"
    "2026-05 Dolomites" "2026-06 Portraits Mara" "2026-07 Faroe Islands" "2026-08 Product Atelier Noor"
    "2026-09 Alps Autumn Scouting" "2026-10 Wedding Lea and Jonas" "2026-10 Fog Valley" "2026-11 Studio Headshots")
  local s i j dir w
  for i in "${!shoots[@]}"; do
    s=${shoots[$i]}
    w=${s:8}; w=${w%% *}
    dir="Pictures/Shoots/2026/$s"
    for j in 0001 0002 0003 0004 0005 0006 0007 0008 0009 0010; do mk "$dir/RAW/IMG_$((i * 40 + 1000 + 10#$j)).cr3"; done
    for j in 0 1 2 3 4 5 6 7 8 9 10 11; do
      img "$dir/Selects/$w-$((j + 1)).jpg" $((i * 3 + j)) $j
      [ "$j" -lt 5 ] && mk "$dir/Edits/$w-$((j + 1)).xmp"
    done
    for j in 0 1 2; do thumb "$dir/Exports/web/$w-web-$((j + 1)).jpg" "$dir/Selects/$w-$((j + 1)).jpg"; done
    mk "$dir/Exports/print/$w-print-1.tif" "$dir/notes.md"
  done

  local c
  for c in "Anna and Tom" "Lea and Jonas" "Atelier Noor" "Nordic Outdoors" "Mara K"; do
    mk "Clients/$c/Contracts/contract.pdf" "Clients/$c/Contracts/model-release.pdf"
    mk "Clients/$c/Invoices/INV-2026-0"{14,27,31}.pdf
    mk "Clients/$c/brief.md"
    img "Clients/$c/Moodboards/mood-1.jpg" $((${#c} + 1)) 1
    img "Clients/$c/Moodboards/mood-2.jpg" $((${#c} + 5)) 2
    img "Clients/$c/Delivered/final-1.jpg" $((${#c} + 3)) 0
    img "Clients/$c/Delivered/final-2.jpg" $((${#c} + 9)) 3
  done

  mk "Catalogs/Main 2026.lrcat" "Catalogs/Main 2025.lrcat" "Catalogs/Previews/"{2026,2025}"/previews.db" "Catalogs/Backups/"{2026-10,2026-11}".zip"
  mk Presets/{"Moody Forest","Soft Film","Golden Hour","Blue Hour","Black and White Grain"}.xmp
  mk Gear/Manuals/{"EOS R5","RF 24-70","RF 70-200","Godox AD200"}.pdf Gear/{"Lens notes","Packing checklist"}.md
  local k=0
  for j in Landscapes Weddings Portraits; do
    for k in 0 1 2 3; do img "Portfolio/$j/$j-$((k + 1)).jpg" $((k * 4 + ${#j})) $k; done
  done
  mk Prints/2026-spring/{lofoten-a2,dolomites-a3,fog-valley-a1}.tif Prints/2026-spring/order-confirmation.pdf
  mk Documents/{Invoices-template.docx,Pricing-2026.pdf,Insurance.pdf,Tax-notes.md}
  mk Downloads/{lightroom-update,capture-one-trial}.dmg Downloads/film-grain-pack.zip
  mk Desktop/{shot-list.md,idea-board.md}

  local n
  for n in 01 02 03 04 05 06 07 08 09 10 11 12 13 14 15 16 17 18 19 20; do mk "Volumes/EOS R5 Card/DCIM/100EOS5D/IMG_$((7000 + 10#$n)).CR3"; done
  mk "Volumes/EOS R5 Card/MISC/"{DSCF.CTG,M0001.CTG}
  for n in 2022 2023 2024 2025; do mk "Volumes/Archive SSD/Shoots $n/"{Spring,Summer,Autumn,Winter}"/"{notes.md,contact-sheet.tif}; done
  mk "Volumes/Backup RAID/Photos-mirror/"{2024,2025,2026}"/mirror.zip" "Volumes/Backup RAID/Catalog-backups/"{2025,2026}".zip"
}

# ======================================================================================
writer() {
  local h=$parent/Demo-writer
  rm -rf "$h"; mkdir -p "$h"; cd "$h"

  local m="Manuscripts/The Lantern Year"
  mk "$m/00 Notes/"{Premise,Characters,Timeline,Outline,"Style sheet"}.md
  local ch=("The Ferry" "Salt" "Lamp Oil" "The Keeper's Daughter" "Fog Signals" "Letters Unsent" "Low Tide" "The Missing Week"
    "A Visitor" "Storm Warning" "Bread and Tar" "The Logbook" "What the Radio Said" "Winter Stores" "The Long Dark"
    "First Light" "The Relief Boat" "Lantern Year")
  local i
  for i in "${!ch[@]}"; do mk "$m/Chapters/$(printf '%02d' $((i + 1))) ${ch[$i]}.md"; done
  mk "$m/Drafts/v1/"{01,02,03,04,05,06}.md "$m/Drafts/v2/"{01,02,03,04,05,06,07,08,09,10}.md "$m/Drafts/Lantern Year v3.docx" "$m/Drafts/Lantern Year v3.pdf"
  img "$m/Cover/front.jpg" 13 0
  img "$m/Cover/back.jpg" 8 3
  mk "$m/Cover/spine.psd" "$m/Cover/blurb.md"

  mk "Manuscripts/Short Stories/"{Salt,"The Tuner","Night Shift","Paper Boats","Small Hours"}.md
  mk "Manuscripts/Short Stories/Salt.docx" "Manuscripts/Short Stories/Paper Boats.docx"
  for i in 2024 2025 2026; do mk "Essays/$i/"{"On slow mornings","The case for notebooks","Why I rewrite","Small towns","Letters to myself","Edits"}.md; done

  mk Research/Lighthouses/{history,"keeper diaries","construction notes",glossary}.md Research/Lighthouses/{Stevenson-1880,"Light List 1932","Keepers of the Coast"}.pdf
  mk Research/Interviews/{"Ingrid, retired keeper","Harbour master","Boatwright"}.m4a Research/Interviews/{"Ingrid transcript","Harbour master transcript"}.md
  mk Research/Articles/{tides,fog,lamps,"weather lore","coastal trade","fresnel lenses","storm of 1953","island schools","ferry timetables","seabirds"}.pdf
  img Research/Maps/coast-chart.jpg 5 2
  img Research/Maps/island-map.jpg 9 0
  img Research/Maps/harbour.jpg 2 3

  local j
  for j in "Salt Review" "Northlight Quarterly" "The Harbour Prize" "Longform Mag" "Tern Press"; do
    mk "Submissions/$j/"{"cover letter",response}.md "Submissions/$j/submission.docx"
  done

  local mo d
  for mo in 01 02 03 04 05 06 07 08 09 10 11; do
    for d in 03 09 14 21 27; do mk "Journal/2026/$mo/2026-$mo-$d.md"; done
  done
  for mo in 01 02 03 04 05 06 07 08 09 10 11 12; do for d in 05 19; do mk "Journal/2025/$mo/2025-$mo-$d.md"; done; done

  for mo in 01 02 03 04 05 06 07 08 09 10 11 12; do for d in 04 11 18 25; do mk "Journal/2024/$mo/2024-$mo-$d.md"; done; done
  for i in 01 02 03 04 05 06 07 08 09 10 11 12 13 14 15 16; do mk "Ideas/idea-$i.md"; done
  mk Ideas/{"Story seeds","Titles I like","Overheard","Opening lines","Endings"}.md
  mk Reading/Fiction/{"The Remains of the Day","Gilead","Piranesi","Ducks, Newburyport","Small Things Like These"}.epub
  mk Reading/Craft/{"On Writing","Bird by Bird","Steering the Craft","The Elements of Style"}.epub
  mk Reading/Poetry/{"Collected Poems","Late Wisconsin Poems"}.pdf
  for i in 01 02 03 04 05 06 07 08 09 10 11 12; do mk Newsletter/issue-0$i.md; done
  mk Documents/{"Writing goals 2026","Query tracker","Residency applications"}.md Documents/Tax-2025.pdf Documents/"Agent contract.pdf"
  mk Downloads/{Scrivener-3,iA-Writer}.dmg Downloads/{residency-guidelines,grant-form}.pdf
  mk Desktop/{"today.md","word-count.csv"}

  mk "Volumes/Backup stick/Lantern Year/"{v1,v2,v3}.zip "Volumes/Backup stick/Journal-2025.zip"
  mk "Volumes/Typewriter archive/scans/"{page-001,page-002,page-003,page-004}.tif "Volumes/Typewriter archive/index.md"
}

# ======================================================================================
producer() {
  local h=$parent/Demo-producer
  rm -rf "$h"; mkdir -p "$h"; cd "$h"

  local t
  for t in "Night Drive" "Glass Rain" "Low Tide" "Static Bloom" "Paper Moons" "Afterhours" "Tape Hiss" "Violet Hour"; do
    mk "Projects/$t/$t.als" "Projects/$t/notes.md"
    mk "Projects/$t/Samples/"{kick,snare,hat,bass-loop,pad-swell}.wav
    mk "Projects/$t/Stems/"{drums,bass,synth,vocal,fx}.wav
    mk "Projects/$t/Bounces/"{v1,v2,final}.mp3 "Projects/$t/Bounces/final.wav"
    mk "Projects/$t/MIDI/"{chords,bassline,drums}.mid
  done

  local b
  for b in 01 02 03 04 05 06 07 08 09 10 11 12; do mk "Sample Library/Drums/Kicks/kick-$b.wav"; done
  for b in 01 02 03 04 05 06 07 08 09 10; do mk "Sample Library/Drums/Snares/snare-$b.wav" "Sample Library/Drums/Hats/hat-$b.wav"; done
  for b in 01 02 03 04 05 06; do mk "Sample Library/Drums/Claps/clap-$b.wav"; done
  for b in 01 02 03 04 05 06 07 08; do mk "Sample Library/Drums/Loops/loop-$((80 + 5 * 10#$b))bpm.wav"; done
  mk "Sample Library/Synths/Pads/"{warm,glass,tape,choir,drift}.wav "Sample Library/Synths/Leads/"{saw,pluck,bell,fm}.wav "Sample Library/Synths/Bass/"{sub,reese,acid,fm}.wav
  mk "Sample Library/FX/Risers/"{r1,r2,r3,r4}.wav "Sample Library/FX/Impacts/"{i1,i2,i3}.wav "Sample Library/FX/Foley/"{rain,vinyl,door,footsteps,tape-stop}.wav
  mk "Sample Library/Vocals/Phrases/"{ooh,ahh,whisper,count}.wav "Sample Library/Vocals/Chops/"{c1,c2,c3,c4,c5,c6}.wav

  local m
  for m in 2026-08 2026-09 2026-10 2026-11; do mk "Mixdowns/2026/$m-"{demo,rough,master}".mp3" "Mixdowns/2026/$m-master.wav"; done
  mk "Releases/EP Glass Rain/masters/"{01-glass-rain,02-low-tide,03-static-bloom,04-paper-moons}.wav
  mk "Releases/EP Glass Rain/press/"{bio.md,one-sheet.pdf,credits.md}
  img "Releases/EP Glass Rain/artwork/cover.jpg" 12 1
  img "Releases/EP Glass Rain/artwork/back.jpg" 4 3
  mk "Releases/EP Glass Rain/artwork/cover.psd"
  mk "Gigs/2026-12 Club Atlas/"{rider.pdf,setlist.md,"stage plot.pdf"} "Gigs/2027-01 Warehouse Sessions/"{rider.pdf,setlist.md} "Gigs/2026-10 Radio Show/"{setlist.md,recording.wav}
  mk MIDI/Chords/{minor-7ths,sus-pads,jazz-ii-v-i,gospel}.mid MIDI/Basslines/{walking,octaves,syncopated}.mid MIDI/Drums/{breakbeat,four-on-floor,halftime,shuffle}.mid
  mk Plugins/Presets/{warm-pad,glass-lead,sub-bass,tape-delay,plate-reverb,sidechain}.fxp
  mk Documents/{"Release plan","Mixing notes","Gear list"}.md Documents/Contract-label.pdf
  mk Downloads/{Ableton-Live-12,Serum-2-update}.dmg Downloads/drum-kit-vol2.zip
  mk Desktop/{"track ideas.md","bpm chart.csv"}

  mk "Volumes/Session Drive/Projects-live/"{"Night Drive","Glass Rain"}"/"{audio.zip,"backup.als"}
  mk "Volumes/Sample Library SSD/Splice/"{Drums,Synths,Vocals}"/"{pack-01.zip,pack-02.zip}
  mk "Volumes/Studio Backup/"{2025,2026}"/"{projects.zip,samples.zip}
}

# Folders get fixed dates too (they would all read "Today" otherwise).
settle() {
  find "$1" -type d -not -path '*/.git' -not -path '*/.git/*' | while IFS= read -r d; do touch -t "$(when "$d")" "$d"; done
}

for p in $want; do
  echo "building $p"
  "$p"
  settle "$parent/Demo-$p"
done
echo "built in $parent: $want"
