#!/bin/sh
# postinst of the harbour-shell .deb (runs as root).
# Harbour Launcher was the old name. Its package (if it was installed) is replaced by this one
# (Replaces/Conflicts in the control file), which also removes its harbour-launcher.service from
# /usr/lib/systemd/user. Here, users who had that unit enabled get harbour-shell.service enabled
# instead, and the boot-splash helper (install_system.sh) waits for the new process name.
# ~/.local/share/harbour (settings, IME dictionary, Paint files, feeds) is never touched.
set -e
[ "${1:-}" = configure ] || exit 0

for home in /root /home/*; do
  wants="$home/.config/systemd/user/default.target.wants"
  old="$wants/harbour-launcher.service"
  [ -L "$old" ] || continue
  # Only links to the packaged unit: a per-user install (~/.config/systemd/user/harbour-launcher.service)
  # is migrated by that user's install_user.sh, which also handles its install dir.
  case "$(readlink "$old")" in /usr/lib/systemd/user/harbour-launcher.service) ;; *) continue ;; esac
  ln -sfn /usr/lib/systemd/user/harbour-shell.service "$wants/harbour-shell.service"
  chown -h --reference="$wants" "$wants/harbour-shell.service" 2>/dev/null || true
  rm -f "$old"
  echo "harbour-shell: $home: Harbour Launcher started at boot, so Harbour Shell does now"
  MIGRATED=yes
done

SPLASH=/etc/systemd/system/harbour-splash-release.service
if [ -f "$SPLASH" ] && grep -q harbour_launche "$SPLASH"; then
  sed -i 's/pgrep -x harbour_launche /pgrep -x harbour-shell /; s/Harbour Launcher/Harbour Shell/' "$SPLASH"
  systemctl daemon-reload 2>/dev/null || true
  echo "harbour-shell: boot splash helper now waits for harbour-shell"
fi

if [ "${MIGRATED:-no}" = yes ]; then
  echo "Harbour Shell replaces Harbour Launcher. Start it now: harbourctl try"
else
  echo "Harbour Shell installed. It is NOT enabled yet."
  echo "Try it:   harbourctl try      (next boot still uses the official launcher)"
  echo "Switch:   harbourctl enable   (go back with: harbourctl disable)"
fi
echo "Japanese dictionary (optional, downloaded only if you ask): harbourctl ime-install"
exit 0
