#!/bin/sh
exec qemu-aarch64 -L "/home/hatch/workspace/cardputer-zero-muse/build-launcher/SDK/github_source/static_lib_v0.0.7" -E LD_LIBRARY_PATH=/usr/lib/aarch64-linux-gnu:/usr/lib/aarch64-linux-gnu/pulseaudio -E SDL_VIDEODRIVER=dummy -E SDL_AUDIODRIVER=dummy "$@"
