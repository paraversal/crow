#!/usr/bin/env bash
# Encode a recording from scripts/shot.sh (CROW_SHOT_RECORD=1) for the web: H.264,
# 60 fps, 1280 px wide, no audio, fast start. Warns past 2 MB.
#
#   scripts/shots/encode-clip.sh target/shots/big-folder-finder-dark.mov docs/media/big-folder.mp4
#
# The README shows these as videos uploaded through GitHub's editor (a committed .mp4
# would only be a link); the site and the manual play the committed files. Needs ffmpeg.
set -euo pipefail
in=${1:?usage: encode-clip.sh <in.mov> <out.mp4>}
out=${2:?usage: encode-clip.sh <in.mov> <out.mp4>}

ffmpeg -v error -y -i "$in" -an -vf "fps=60,scale=1280:-2:flags=lanczos" \
  -c:v libx264 -preset slow -crf 22 -pix_fmt yuv420p -movflags +faststart "$out"
bytes=$(stat -f %z "$out")
note=""
[ "$bytes" -le 2097152 ] || note="  (over 2 MB)"
echo "$out $((bytes / 1024)) KB$note"
