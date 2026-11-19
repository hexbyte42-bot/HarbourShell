#!/bin/sh
# Needs root (run: sudo sh install_system.sh). One-time system setup so that Harbour Shell
# is visible after a boot:
#   1. stop the boot splash (fb_load) once Harbour is running;
#   2. show Harbour's own boot picture (boot/splash.bmp: wordmark, version, "for M5Stack Cardputer
#      Zero") instead of the built-in logo, from the next boot on.
#   sudo sh install_system.sh uninstall-splash    puts the built-in logo back (step 2 only).
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
DROPIN=/etc/systemd/system/fb_load.service.d/harbour.conf
if [ "${1:-}" = "uninstall-splash" ]; then
  rm -f "$DROPIN"
  rmdir /etc/systemd/system/fb_load.service.d 2>/dev/null || true
  systemctl daemon-reload
  echo "done: the built-in boot logo is back from the next boot"
  exit 0
fi
install -m 644 "$HERE/harbour-splash-release.service" /etc/systemd/system/harbour-splash-release.service
if [ -f "$HERE/boot/splash.bmp" ]; then
  install -D -m 644 "$HERE/boot/splash.bmp" /usr/local/share/harbour/splash.bmp
  install -D -m 644 "$HERE/fb_load-harbour.conf" "$DROPIN"
fi
systemctl daemon-reload
systemctl enable harbour-splash-release.service
# Right now too (this boot): release the splash.
systemctl stop fb_load.service || true
systemctl restart harbour-splash-release.service || true
echo "done: the boot splash no longer covers Harbour"
[ -f "$HERE/boot/splash.bmp" ] && echo "Harbour's boot picture shows from the next boot"
