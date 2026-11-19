#!/bin/sh
# Build harbour-shell for the Cardputer Zero (Debian 13 trixie, aarch64) inside Docker.
# colima does not mount this volume, so the source goes in through stdin and the
# binaries come back through stdout.   Usage: ./tools_build_arm64.sh  -> dist/arm64/{harbour-shell,harbour_probe}
set -eu
cd "$(dirname "$0")"
mkdir -p dist/arm64
PARENT=$(cd .. && pwd)
HERE=$(basename "$(pwd)")
tar cf - -C "$PARENT" --exclude='*/target' --exclude='*/dist' --exclude='*/.git' --exclude='*/dev' \
  --exclude="$HERE/memos" --exclude="$HERE/internal" "$HERE" |
docker run -i --rm \
  -v harbour-target:/target \
  -v harbour-cargo:/usr/local/cargo/registry \
  -e CARGO_TARGET_DIR=/target \
  -e HERE="$HERE" \
  rust:trixie sh -c '
    set -eu
    mkdir /src && tar xf - -C /src && cd "/src/$HERE"
    apt-get update -qq >&2
    apt-get install -y -qq libsdl2-dev libsdl2-image-dev pkg-config >&2
    cargo build --release --locked --bin harbour-shell --bin harbour_probe --bin harbour_drmprobe --bin harbour_ime_dict >&2
    cargo test --release --locked --lib 2>&1 | tail -4 >&2
    tar cf - -C /target/release harbour-shell harbour_probe harbour_drmprobe harbour_ime_dict
  ' | tar xf - -C dist/arm64
chmod +x dist/arm64/harbour-shell dist/arm64/harbour_probe dist/arm64/harbour_drmprobe dist/arm64/harbour_ime_dict
file dist/arm64/harbour-shell
