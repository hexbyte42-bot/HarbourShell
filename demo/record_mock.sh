#!/bin/sh
# Record the demo on the Mac with the mock backend (no device involved).
# usage: demo/record_mock.sh [script=demo/main.script] [out=demo/out/demo.mp4] [scale=4]
set -eu
cd "$(dirname "$0")/.."
script=${1:-demo/main.script}; out=${2:-demo/out/demo.mp4}; scale=${3:-4}
work=${TMPDIR:-/tmp}/harbour-demo-$$
mkdir -p "$work/home/Documents" "$work/home/Music" "$work/home/Pictures" "$(dirname "$out")"
printf "Harbour demo\n" > "$work/home/Documents/readme.txt"; printf "todo\n" > "$work/home/Documents/notes.txt"
[ -x target/release/harbour-shell ] || cargo build --release --bin harbour-shell
# A throw-away config/data dir so the demo always starts from the same state and never touches ~/.config.
HOME="$work/home" HARBOUR_MOCK=1 HARBOUR_HEADLESS=1 HARBOUR_CONFIG="$work/config.json" HARBOUR_DATA="$work/data" \
HARBOUR_ORDER_FILE="$work/order.txt" HARBOUR_SHOT_DIR="$work" \
  target/release/harbour-shell --demo "$script" --rec "$work/rec" --rec-fps "${HARBOUR_REC_FPS:-15}"
tools/rec_to_mp4.sh "$work/rec" "$out" "$scale"
rm -rf "$work"
