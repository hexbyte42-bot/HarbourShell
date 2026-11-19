#!/bin/sh
# Install Harbour Shell for the current user only (no sudo needed).
# A fresh install is NOT enabled at boot; use `harbourctl try` to run it and `harbourctl enable`
# to make it the launcher. The official APPLaunch is not touched.
#   usage: install_user.sh <bundle-dir>
#
# Upgrading from Harbour Launcher (the old name: ~/harbour-launcher, harbour-launcher.service) is
# automatic: the new unit is installed first, then the old one is stopped, disabled and removed.
# If the old unit started at boot, the new one does. ~/.local/share/harbour (settings, IME
# dictionary, Paint files, feeds) is shared by both names and is not touched.
set -eu
SRC=${1:?usage: install_user.sh <bundle-dir>}
SRC=$(cd "$SRC" && pwd)
DEST="$HOME/harbour-shell"
UNIT=harbour-shell.service
UD="$HOME/.config/systemd/user"
OLD_DEST="$HOME/harbour-launcher"
OLD_UNIT=harbour-launcher.service
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
sc() { systemctl --user "$@"; }

# 1. What the old install looked like (before anything changes).
MIGRATE=no; OLD_ENABLED=no; OLD_RUNNING=no
if [ -e "$UD/$OLD_UNIT" ] || [ -d "$OLD_DEST" ]; then
  MIGRATE=yes
  sc is-enabled --quiet "$OLD_UNIT" 2>/dev/null && OLD_ENABLED=yes
  sc is-active --quiet "$OLD_UNIT" 2>/dev/null && OLD_RUNNING=yes
  echo "found Harbour Launcher (old name): starts at boot: $OLD_ENABLED, running: $OLD_RUNNING"
fi

# 2. Install the new files and unit.
mkdir -p "$DEST" "$UD" "$HOME/.local/bin"
# Remove legacy top-level fonts/ from older installs.
rm -rf "$DEST/bin" "$DEST/assets" "$DEST/apps" "$DEST/fonts"
cp -r "$SRC/bin" "$SRC/assets" "$SRC/apps" "$DEST/"
sed -e "s#/usr/share/harbour-shell#$DEST#g" "$SRC/$UNIT" > "$UD/$UNIT"
install -m755 "$SRC/harbourctl" "$HOME/.local/bin/harbourctl"
install -m755 "$SRC/ime_install.sh" "$DEST/ime_install.sh"
# Your own drop-ins for the old unit (e.g. extra Environment=) carry over, with the paths renamed.
if [ -d "$UD/$OLD_UNIT.d" ] && [ ! -e "$UD/$UNIT.d" ]; then
  mkdir -p "$UD/$UNIT.d"
  for f in "$UD/$OLD_UNIT.d"/*.conf; do
    [ -f "$f" ] || continue
    sed -e "s#$OLD_DEST#$DEST#g" -e "s#/usr/share/harbour-launcher#/usr/share/harbour-shell#g" \
        -e "s#bin/harbour_launcher#bin/harbour-shell#g" "$f" > "$UD/$UNIT.d/$(basename "$f")"
  done
  echo "copied drop-ins: $UD/$UNIT.d"
fi
sc daemon-reload
echo "installed to $DEST"

# 3. Retire the old names, now that the new unit is in place.
if [ "$MIGRATE" = yes ]; then
  [ "$OLD_ENABLED" = yes ] && sc enable "$UNIT" && echo "Harbour Shell starts at boot (as Harbour Launcher did)"
  sc disable "$OLD_UNIT" 2>/dev/null || true
  # The old service is this script's parent when it runs in Harbour's own Terminal: stopping it
  # would end this script, so in that case the switch happens at the next boot / `harbourctl try`.
  INSIDE=no
  grep -q "$OLD_UNIT" /proc/self/cgroup 2>/dev/null && INSIDE=yes
  if [ "$INSIDE" = no ]; then
    sc stop "$OLD_UNIT" 2>/dev/null || true
    rm -f "$UD/$OLD_UNIT"
    rm -rf "$UD/$OLD_UNIT.d"
    sc daemon-reload
    sc reset-failed "$OLD_UNIT" 2>/dev/null || true
  else
    # Keep the running unit's file until it has stopped; Conflicts= in the new unit stops it.
    echo "running inside Harbour Launcher: the old unit file is removed by 'harbourctl try' or the next install"
  fi
  # The old install dir: what came from a bundle goes, anything else is kept inside the new dir.
  if [ -d "$OLD_DEST" ] && [ "$INSIDE" = no ]; then
    rm -rf "$OLD_DEST/bin" "$OLD_DEST/assets" "$OLD_DEST/apps" "$OLD_DEST/fonts" "$OLD_DEST/ime_install.sh"
    if rmdir "$OLD_DEST" 2>/dev/null; then echo "removed $OLD_DEST"
    else mv "$OLD_DEST" "$DEST/from-harbour-launcher" && echo "kept leftovers in $DEST/from-harbour-launcher"; fi
  fi
  # Cache (browser history, last app log).
  if [ -d "$HOME/.cache/harbour-launcher" ] && [ ! -e "$HOME/.cache/harbour-shell" ]; then
    mv "$HOME/.cache/harbour-launcher" "$HOME/.cache/harbour-shell"
  fi
  [ "$OLD_RUNNING" = yes ] && [ "$INSIDE" = no ] && sc start "$UNIT" && echo "Harbour Shell is running"
  # The boot-splash helper (installed by install_system.sh, needs root) waits for the old process name.
  if grep -q harbour_launche /etc/systemd/system/harbour-splash-release.service 2>/dev/null; then
    echo "NOTE: the boot splash helper still waits for the old name. Run once:"
    echo "  sudo sh $SRC/install_system.sh"
  fi
fi

# Optional Japanese input dictionary: never bundled; asked here (default No) and downloaded only on "yes".
# Declining or a failed download leaves kana-only input. Later: harbourctl ime-install
# An existing dictionary in ~/.local/share/harbour/ime is kept as it is.
if [ -f "${HARBOUR_IME_DIR:-$HOME/.local/share/harbour/ime}/harbour-ime.dic" ]; then
  echo "Japanese dictionary already installed (kept)"
else
  sh "$DEST/ime_install.sh" install || echo "(no dictionary installed; run 'harbourctl ime-install' to retry)"
fi
