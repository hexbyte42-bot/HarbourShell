#!/bin/sh
# Assemble dist/bundle (what install_user.sh installs) from dist/arm64/harbour-shell.
set -eu
cd "$(dirname "$0")"
B=dist/bundle
rm -rf "$B"; mkdir -p "$B/bin"
cp dist/arm64/harbour-shell "$B/bin/"
# Dictionary generator only (no dictionary data): used by ime_install.sh if the user opts in.
cp dist/arm64/harbour_ime_dict "$B/bin/"
cp -r assets "$B/assets"
# Built-in app packages (apps/<id>/: .desktop, icon.png, res/), next to assets/.
cp -r apps "$B/apps"
find "$B/apps" -name .DS_Store -delete
# Artwork sources / working files are kept private and are never shipped.
rm -rf "$B/assets/icons-src" "$B/assets/statusbar-src" "$B/assets/.DS_Store" "$B/assets/images/.DS_Store" "$B/assets/images/icons/.DS_Store"
rm -f "$B"/assets/images/icons/Test*.png
cp LICENSE NOTICE THIRD_PARTY.md THIRD_PARTY_LICENSES.md "$B/"
cp packaging/harbour-shell.service packaging/harbourctl packaging/install_user.sh packaging/harbour-splash-release.service packaging/install_system.sh packaging/ime_install.sh packaging/fb_load-harbour.conf "$B/"
# Boot picture for fb_load (built on this machine: wordmark + version + "for M5Stack Cardputer Zero").
mkdir -p "$B/boot"
cargo run -q --bin boot_splash -- assets/themes/theme-black.yaml "$B/boot/splash.bmp"
du -sh "$B"
