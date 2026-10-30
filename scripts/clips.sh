#!/usr/bin/env bash
# The test clips prep is checked on, built from sources anyone can fetch, into .cache/clips/:
#
#   bedroom.mp4  EdgeTAM's bedroom video (Apache-2.0), 200 frames at 24 fps: two children, one shot.
#   surf6.mp4    6 s of "Die Ewige Welle und Surfers in Augsburg" (Sillerkiil, CC0, Wikimedia Commons).
#   cuts.mp4     bedroom 0–71, surf 0–71, bedroom 120–199 at 24 fps: cuts after frames 71 and 143 (47 and 95 at
#                16 fps), and both children on either side of the surfing shot.
#
#     scripts/clips.sh              (needs .models/ffmpeg: cargo run -p kawarimi-assets --bin kawarimi-fetch -- ffmpeg)
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ff="$root/.models/ffmpeg/ffmpeg"
out="$root/.cache/clips"
mkdir -p "$out" "$root/.cache"
encode=(-c:v h264_nvenc -preset p5 -cq 18 -pix_fmt yuv420p)
"$ff" -v error -f lavfi -i color=size=256x256 -frames:v 1 -c:v h264_nvenc -f null - 2>/dev/null || encode=(-c:v libopenh264 -b:v 12M -pix_fmt yuv420p)

if [[ ! -d "$root/.cache/EdgeTAM/notebooks/videos/bedroom" ]]; then
    git clone -q --filter=blob:none --no-checkout https://github.com/facebookresearch/EdgeTAM.git "$root/.cache/EdgeTAM"
    git -C "$root/.cache/EdgeTAM" sparse-checkout set --no-cone notebooks/videos/bedroom
    git -C "$root/.cache/EdgeTAM" checkout -q 7711e012a30a2402c4eaab637bdb00a521302c91
fi
if [[ ! -f "$out/surf.ogv" ]]; then
    curl -fsSL -A "kawarimi test clips" -o "$out/surf.ogv" \
        "https://upload.wikimedia.org/wikipedia/commons/d/df/Die_Ewige_Welle_und_Surfers_in_Augsburg.ogv"
fi
echo "f7eed85bcbbf7ebaeadaf27a791761a96dd9ba92  $out/surf.ogv" | sha1sum -c --quiet

"$ff" -v error -y -framerate 24 -i "$root/.cache/EdgeTAM/notebooks/videos/bedroom/%05d.jpg" "${encode[@]}" "$out/bedroom.mp4"
"$ff" -v error -y -ss 20 -t 6 -i "$out/surf.ogv" -an -vf "fps=24,scale=-2:540" "${encode[@]}" "$out/surf6.mp4"
"$ff" -v error -y -i "$out/bedroom.mp4" -i "$out/surf6.mp4" -filter_complex \
    "[0:v]trim=start_frame=0:end_frame=72,setpts=PTS-STARTPTS,scale=960:540,setsar=1[a];\
     [1:v]trim=start_frame=0:end_frame=72,setpts=PTS-STARTPTS,scale=960:540,setsar=1[b];\
     [0:v]trim=start_frame=120:end_frame=200,setpts=PTS-STARTPTS,scale=960:540,setsar=1[c];\
     [a][b][c]concat=n=3:v=1:a=0[v]" -map "[v]" "${encode[@]}" "$out/cuts.mp4"
echo "clips in $out"
