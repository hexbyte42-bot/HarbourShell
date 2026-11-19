#!/bin/sh
# Raw frames from `harbour-shell --rec DIR` -> an integer-scaled mp4 (nearest neighbour keeps the dots crisp).
# usage: tools/rec_to_mp4.sh REC_DIR_OR_FRAMES_FILE OUT.mp4 [SCALE=4] [FPS=15]
set -eu
src=${1:?rec dir or frames.rgb565}; out=${2:?out.mp4}; scale=${3:-4}; fps=${4:-}
if [ -d "$src" ]; then
  [ -n "$fps" ] || fps=$(sed -n 's/^fps=//p' "$src/rec.txt" 2>/dev/null || true)
  src="$src/frames.rgb565"
fi
fps=${fps:-15}
w=$((320 * scale)); h=$((170 * scale))
# -sws_flags neighbor: no smoothing. yuv420p + even size: plays everywhere. High bitrate/crf: flat 1-bit areas band easily.
ffmpeg -loglevel error -y -f rawvideo -pix_fmt rgb565le -video_size 320x170 -framerate "$fps" -i "$src" \
  -vf "scale=${w}:${h}:flags=neighbor,format=yuv420p" -c:v libx264 -preset slow -crf 14 -movflags +faststart "$out"
echo "wrote $out (${w}x${h} @ ${fps} fps)"
